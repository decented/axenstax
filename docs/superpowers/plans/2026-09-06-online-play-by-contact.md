# Online Play by Contact — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a player host a world at home and have a friend in another house join it by picking a contact (a Signet persona npub), with the game traffic going directly house-to-house over the existing QUIC transport.

**Architecture:** Nostr relays carry only an encrypted offer/answer handshake between two per-install **runtime keys** that the players' Signet personas have each attested once. Both sides bind one UDP socket, gather reachability candidates (LAN, IPv6, UPnP, STUN), exchange them inside NIP-44 ciphertext, punch through their routers, then hand that same socket to quinn. The existing persona-signed kind-21236 QUIC join handshake runs unchanged on top. No AxeNStax server sits in the game path; no relay learns anything about the world.

**Tech Stack:** Rust (native only — `#[cfg(not(target_arch = "wasm32"))]`), `nostr 0.44.3` (`nip59` + `nip44`), `tokio-tungstenite` for relays, `quinn 0.11` for QUIC, `igd-next 0.17.1` for UPnP (the one new crate), hand-rolled RFC 5389 STUN.

**Spec:** `docs/superpowers/specs/2026-09-06-online-play-by-contact-design.md` — read it alongside this plan. Every requirement in §2–§9 maps to a task below.

## Global Constraints

Every task's requirements implicitly include this section.

- **Native only.** Every new module carries `#![cfg(not(target_arch = "wasm32"))]` as its first line (after `//!` docs where the file leads with them — the `#![...]` inner attribute must precede items but may follow `//!` doc comments; `contacts.rs` shows the exact shape). **A free function is NOT covered by its caller's `#[cfg]`** — if a wasm build can reach a symbol, it must compile there or be gated. `trunk build` in `check.sh` is what proves this.
- **Web is the anonymous local taster.** Nothing in this feature may appear in the wasm bundle. Task 1 adds `axenstax://invite/` to `tools/smoke/forbidden-symbol.mjs` so the gate proves it.
- **Red lines (CLAUDE.md).** No public directory; no AxeNStax relay in the game-traffic path; no central collection of kids' data (candidate IPs travel only inside NIP-44 ciphertext, the persona↔runtime attestation is never published); never positioned as a social network. Copy stays sovereignty/co-building.
- **`PROTOCOL_VERSION` does not change.** No packet changes. It is currently `61` (`src/protocol.rs:986`). The `protocol` field inside an Offer/Answer exists so a mismatch is explained *before* a connect is attempted.
- **`HostedServer.require_signin` stays `true`** for the QUIC path. The online host's allowlist is contacts at Kin/Kith plus this session's bearer-admitted personas, fed through the existing `HostedServer::set_access_policy`.
- **Default relay list** (spec §6), in this order: `wss://relay.trotters.cc` **[test-only — remove before public launch]**, `wss://nos.lol`, `wss://relay.damus.io`, `wss://relay.primal.net`.
- **Default invite expiry: 48 hours.** Bearer: 16 bytes. Max 8 relays in an invite; every relay URL must start with `wss://`.
- **Signalling kinds:** `20900` = `join-offer` (joiner → host), `20901` = `join-answer` (host → joiner). Both ephemeral, both NIP-44 sealed runtime-key ↔ runtime-key, both `p`-tagged with the recipient's runtime pubkey.
- **Attestation:** kind `30420`, `v` tag `"1"`, plus the new tag `role=player`. `role=server` is implied when the tag is absent.
- **Timing:** offer/answer freshness window ±120 s; connect-race stagger 150 ms; overall join deadline 8 s; punch = 3 datagrams 100 ms apart; UPnP lease 7200 s renewed hourly.
- **Display rule:** render `npub…`, never hex ([[feedback_npub_only_display]]). Hex stays internal.
- **UK English** in all player-facing copy. No money/earning words on any text surface.
- **Every command in this plan runs from the repository root** unless it starts
  with its own `cd`. Paths are repo-relative on purpose — absolute host paths
  must never enter the tree (the OPSEC pre-commit hook rejects them).
- **This laptop crashes when the CPU is maxed.** Every cargo command in this plan is written `CARGO_BUILD_JOBS=4 nice -n 10 cargo …`. Run **one at a time**. Never background two builds. Run `./check.sh` in the **foreground**.
- **`core.filemode=false`.** Commits are path-restricted: `git add <paths> && git commit -m "…" -- <paths>`. Never `git add -A` or `git add .` (a concurrent agent shares this tree).
- **Every commit message ends with this trailer line in the body:**
  ```
  Claude-Session: https://claude.ai/code/session_011gW6q5QFBTbF1BNMHiayce
  ```
- **Concrete, not cards** (CLAUDE.md). Files stay under ~500 lines; anything temporary is marked `// BRIDGE: <what this becomes> — replace when <trigger>`.
- **Never write the private Signet repo's name** anywhere in the tree — refer to its issues as "upstream Signet ticket 187" / "ticket 188". An OPSEC pre-commit hook rejects the literal name.

## Choices this plan makes where the spec left them open

Each is a deliberate decision, stated once here and applied throughout.

1. **The spec's `admission.rs` is named `online_admission.rs`** — `src/admission.rs` already exists (capacity admission, Spec A §6) and must not be disturbed.
2. **`invite.rs` is native-gated** despite the spec's module table calling it "cross-platform, trivially". It is pure and trivially testable either way; gating it keeps the whole online surface out of the web taster's bundle, which red line 1/4 wants.
3. **`mint_attestation` is NOT given a `role` parameter.** It already takes 7 arguments; an 8th trips `clippy::too_many_arguments` under `-D warnings`. `runtime_identity::mint_player_attestation` builds its own tag list instead.
4. **`Tier` gains `serde::Serialize`/`Deserialize`** (lowercase) so the contacts mirror can round-trip it. It does **not** gain `Ord` — admission uses `matches!(t, Tier::Kin | Tier::Kith)`, not a comparison.
5. **`rendezvous/` has four files**, not the spec's three: `mod.rs`, `payload.rs`, `verify.rs`, `relay_client.rs`. Splitting verification out keeps each under ~300 lines.
6. **`RemoteTransport` stays `Copy` and unchanged.** `HostedServer::start` delegates to a private `start_inner(…, prebound: Option<std::net::UdpSocket>)`; the new public entry point is `HostedServer::start_online`.
7. **The host keeps a `try_clone()` of the UDP socket** before handing the original to quinn, so it can send punch datagrams from the same source port. quinn silently drops the inbound non-QUIC punches.
8. **`connect_to_server_on_socket` returns the race outcome on a channel** (`OnlineConnect { transport, outcome }`) because the caller must show the "couldn't reach" copy when no candidate wins.
9. **The Online panel is opened by `/online`**, not by a new key binding (no keybinding collision to guess at). Starting an online host copies the invite link to the clipboard and toasts.
10. **The IPv6 STUN decode test is built synthetically** (XOR a known address with cookie‖txid in the test, assert the parser recovers it) rather than pasting an IPv6 RFC vector. The IPv4 RFC 5769 vectors are pasted verbatim.
11. **A player attestation carries `name = ""` and no `host` tag** — a player attestation names no server. Validity 90 days, `allowed_kinds = [20900, 20901]`.

## File Structure

**New (all native-gated):**

| File | Responsibility |
|---|---|
| `game/engine/src/invite.rs` | The `axenstax://invite/2` link/QR format: mint, render, parse, expiry, bearer. Pure. |
| `game/engine/src/runtime_identity.rs` | Runtime keypair mint/load; `role=player` attestation mint (bunker), verify, store. |
| `game/engine/src/online_admission.rs` | The admission rule: contact tier × bearer state → admit/refuse. Pure. |
| `game/engine/src/rendezvous/mod.rs` | Module root + re-exports. |
| `game/engine/src/rendezvous/payload.rs` | `Offer`/`Answer`/`Candidate` types + NIP-44 seal/open. |
| `game/engine/src/rendezvous/verify.rs` | The verification chain + replay guard. Pure. |
| `game/engine/src/rendezvous/relay_client.rs` | `RendezvousRelay` trait, the in-memory fake, the multi-relay worker. |
| `game/engine/src/nat/mod.rs` | Module root. |
| `game/engine/src/nat/stun.rs` | RFC 5389 Binding Request/Response codec. Pure + a live helper. |
| `game/engine/src/nat/candidates.rs` | Candidate kinds, ordering, gathering. |
| `game/engine/src/nat/upnp.rs` | `igd-next` port mapping + lease renewal. |
| `game/engine/src/nat/punch.rs` | Punch datagrams + the pure connect-race state machine. |
| `game/engine/src/online_host.rs` | Host-side orchestration (spec §5.1). |
| `game/engine/src/online_join.rs` | Joiner-side orchestration + the §4.4 failure copy (spec §5.2). |
| `game/engine/src/friends_ui.rs` | "Friends & servers" column, Online panel, "Your address" panel. |
| `game/engine/src/commands/builtins/online.rs` | `/online` — toggle the panel, print the invite link. |
| `game/engine/src/test_integration/online_play.rs` | In-process host↔joiner integration tests over a fake relay. |
| `docs/player-guide/play-with-a-friend-online.md` | Player-facing how-to. |
| `docs/test-sheets/2026-09-06-online-play.md` | Owner live-test sheet. |

**Modified:**

| File | Change |
|---|---|
| `game/engine/Cargo.toml` | `nostr` gains `nip44`; new native dep `igd-next = "0.17.1"`. |
| `game/engine/src/main.rs` | Register the new modules. |
| `game/engine/src/comms.rs` | `Tier` gains serde derives. |
| `game/engine/src/contacts.rs` | `Contact` gains 3 fields; mirror file; union with the Kenspeckle export. |
| `game/engine/src/server_identity/attestation.rs` | `Attestation` gains `role: Option<String>`; `verify_structural` reads the tag. |
| `game/engine/src/network.rs` | Endpoint constructors from a pre-bound socket; the online connect race. |
| `game/engine/src/hosted_server.rs` | `start_inner` + `start_online`; accept thread takes a pre-bound socket. |
| `game/engine/src/remote_client.rs` | `connect_authed_on_transport`. |
| `game/engine/src/graphics_settings.rs` | `online_relays`, `online_port` + the Settings → Online heading. |
| `game/engine/src/menu.rs` | New `MenuAction`/`MenuDialog` variants; mount the Friends column. |
| `game/engine/src/game_loop.rs` | Dispatch for the new menu actions; poll `OnlineHost`/`OnlineJoin`. |
| `game/engine/src/commands/{dispatch,builtins/mod}.rs` | `/online` registration + result variants. |
| `game/engine/src/test_integration/mod.rs` | Register `online_play`. |
| `tools/smoke/forbidden-symbol.mjs` | Add `axenstax://invite/`. |
| `docs/spec/04-networking.md` | New §1.9. |
| `docs/operators/host-overview.md` | "Play online with a friend" note. |
| `docs/player-guide/index.md` | Link the new page. |
| `tools/sites/wiki/app.py` | Register the new player-guide page. |

---

## Phase 1 — Identity, contacts, invite

### Task 1: The invite link (`invite.rs`)

**Files:**
- Create: `game/engine/src/invite.rs`
- Modify: `game/engine/src/main.rs` (register the module)
- Modify: `tools/smoke/forbidden-symbol.mjs` (add the scheme marker)

**Interfaces:**
- Consumes: nothing from other tasks. `getrandom` (already a dependency) for the bearer.
- Produces:
  ```rust
  pub const INVITE_VERSION: u32 = 2;
  pub const DEFAULT_INVITE_TTL_SECS: u64 = 48 * 3600;
  pub const MAX_RELAYS: usize = 8;
  pub const BEARER_LEN: usize = 16;

  #[derive(Clone, Debug, PartialEq, Eq)]
  pub struct Invite {
      pub host_persona: String,      // npub (bech32), never hex
      pub host_runtime: [u8; 32],    // x-only pubkey, hex on the wire
      pub relays: Vec<String>,
      pub bearer: [u8; 16],
      pub expires_at: u64,           // unix seconds
      pub world_name: String,
  }
  #[derive(Clone, Debug, PartialEq, Eq)]
  pub enum InviteError {
      BadScheme(String), BadVersion(String), MissingField(&'static str),
      BadField(&'static str), Expired { expires_at: u64, now: u64 },
      RelayNotWss(String), TooManyRelays(usize),
  }
  impl Invite {
      pub fn to_link(&self) -> String;
      pub fn parse(s: &str, now: u64) -> Result<Invite, InviteError>;
  }
  pub fn mint_bearer() -> [u8; 16];
  ```

- [ ] **Step 1: Write the failing tests**

Create `game/engine/src/invite.rs` with only the test module (the file will not compile yet — that is the point of step 2):

```rust
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
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo test --bin axenstax-engine invite:: 2>&1 | tail -20
```
Expected: FAIL — `cannot find type Invite in this scope` / `file not found for module invite` (the module is not registered yet).

- [ ] **Step 3: Write the implementation**

Put this **above** the `#[cfg(test)] mod tests` block in `game/engine/src/invite.rs`:

```rust
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
```

- [ ] **Step 4: Register the module**

In `game/engine/src/main.rs`, next to the other native-gated module declarations (near `mod contacts;` at line ~122), add:

```rust
// Online play by contact (2026-09-06) — the invite link/QR format. Native only:
// the web build is the anonymous local sandbox and carries no online surface.
#[cfg(not(target_arch = "wasm32"))]
mod invite;
```

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo test --bin axenstax-engine invite:: 2>&1 | tail -20
```
Expected: PASS — `test result: ok. 9 passed`.

- [ ] **Step 6: Add the forbidden-symbol marker**

In `tools/smoke/forbidden-symbol.mjs`, add to the `FORBIDDEN` array:

```js
  ["axenstax://invite/", "online play by contact is native only — the web build is the anonymous local taster"],
```

- [ ] **Step 7: Verify clippy is clean**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo clippy -- -D warnings 2>&1 | tail -20
```
Expected: `Finished` with no warnings. (`Invite`/`InviteError` have no production caller yet — if clippy reports `dead_code`, add `#![allow(dead_code)]` immediately after the `#![cfg(...)]` line with the comment `// Consumed by online_host/online_join (Tasks 15-16).`)

- [ ] **Step 8: Commit**

```bash
git add game/engine/src/invite.rs game/engine/src/main.rs tools/smoke/forbidden-symbol.mjs && git commit -m "$(cat <<'EOF'
feat(online): invite link format + parser (P1)

The axenstax://invite/2 link a host mints for one world: persona npub, host
runtime pubkey, relay list, 16-byte bearer, expiry, world name. Pure and
native-gated; the forbidden-symbol gate now proves it stays out of the web
bundle.

Claude-Session: https://claude.ai/code/session_011gW6q5QFBTbF1BNMHiayce
EOF
)" -- game/engine/src/invite.rs game/engine/src/main.rs tools/smoke/forbidden-symbol.mjs
```

---

### Task 2: Runtime key + `role=player` attestation (`runtime_identity.rs`)

**Files:**
- Create: `game/engine/src/runtime_identity.rs`
- Modify: `game/engine/src/server_identity/attestation.rs` (add `role` to `Attestation`, read it in `verify_structural`)
- Modify: `game/engine/src/main.rs` (register the module)

**Interfaces:**
- Consumes (existing, exact signatures — copy them, do not guess):
  ```rust
  // src/server_identity/attestation.rs
  pub const ATTESTATION_KIND: u16 = 30420;
  pub const ATTESTATION_VERSION: &str = "1";
  pub struct Attestation {
      pub event: nostr::Event,
      pub operator: nostr::PublicKey,
      pub server_pubkey: nostr::PublicKey,
      pub valid_from: nostr::Timestamp,
      pub valid_until: nostr::Timestamp,
      pub allowed_kinds: Vec<u16>,
      pub server_name: String,
      pub host_hint: Option<String>,
  }
  pub enum AttestationError {
      BadSignature, WrongServerKey, Expired, NotYetValid, BadKind, BadVersion, Malformed(String),
  }
  pub fn verify_structural(event: &Event, expected_server: &PublicKey) -> Result<Attestation, AttestationError>;
  pub fn verify(event: &Event, expected_server: &PublicKey, now: Timestamp) -> Result<Attestation, AttestationError>;

  // src/native_mailbox/key.rs — the minting + 0600 file pattern this task copies (NOTE 2026-10-01: native_mailbox/key.rs was removed with the reply path; the pattern lives on in the runtime-key code.)
  pub fn load_existing(path: &Path) -> Option<nostr::Keys>;
  pub fn load_or_mint(path: &Path) -> Result<nostr::Keys, String>;

  // src/signet/native_signer.rs — the bunker the attestation is signed by
  pub fn restore_signer() -> Result<Option<signet_nip46_client::BunkerSession>, String>;
  pub fn current_owner_pubkey() -> Option<String>;   // persona pubkey hex, or None for a guest
  ```
- Produces:
  ```rust
  // src/server_identity/attestation.rs — Attestation gains one field
  pub role: Option<String>,     // Some("player") for a player attestation; None ⇒ server

  // src/runtime_identity.rs
  pub const PLAYER_ROLE: &str = "player";
  pub const PLAYER_ATTESTATION_KINDS: [u16; 2] = [20900, 20901];
  pub const PLAYER_DELEGATION_DAYS: u64 = 90;
  pub fn runtime_key_path() -> std::path::PathBuf;         // profile/runtime_key.json
  pub fn attestation_path() -> std::path::PathBuf;         // profile/runtime_attestation.json
  pub fn load_or_mint_runtime(path: &Path) -> Result<nostr::Keys, String>;
  pub fn load_attestation(path: &Path) -> Option<nostr::Event>;
  pub fn store_attestation(path: &Path, ev: &nostr::Event) -> Result<(), String>;
  pub async fn mint_player_attestation<S: nostr::NostrSigner>(
      persona: &S, runtime: &nostr::PublicKey, now: nostr::Timestamp, validity_days: u64,
  ) -> Result<nostr::Event, String>;
  pub fn verify_player_attestation(
      ev: &nostr::Event, expected_runtime: &nostr::PublicKey, now: nostr::Timestamp,
  ) -> Result<nostr::PublicKey, AttestationError>;   // Ok = the attesting persona
  pub struct RuntimeIdentity { /* private */ }
  impl RuntimeIdentity {
      pub fn load() -> Result<Option<RuntimeIdentity>, String>;
      pub fn keys(&self) -> &nostr::Keys;
      pub fn runtime_pubkey(&self) -> nostr::PublicKey;
      pub fn attestation(&self) -> Option<&nostr::Event>;
      pub fn persona(&self, now: nostr::Timestamp) -> Option<nostr::PublicKey>;
  }
  ```

- [ ] **Step 1: Write the failing test for the `role` tag**

Append to the `mod tests` block at the bottom of `game/engine/src/server_identity/attestation.rs`:

```rust
    #[tokio::test]
    async fn role_tag_is_read_and_absent_means_server() {
        let op = Keys::generate();
        let server = Keys::generate().public_key();
        // A server attestation carries no `role` tag at all.
        let ev = signed(&op, &server, 1_000, 2_000, ATTESTATION_VERSION).await;
        assert_eq!(verify_structural(&ev, &server).unwrap().role, None);

        // A player attestation carries role=player and still verifies.
        let mut tags = ev.tags.iter().map(|t| t.clone()).collect::<Vec<_>>();
        tags.push(Tag::parse(["role", "player"]).unwrap());
        let player_ev = EventBuilder::new(Kind::Custom(ATTESTATION_KIND), "")
            .tags(tags)
            .sign(&op)
            .await
            .unwrap();
        assert_eq!(
            verify_structural(&player_ev, &server).unwrap().role.as_deref(),
            Some("player")
        );
    }
```

- [ ] **Step 2: Run it to verify it fails**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo test --bin axenstax-engine role_tag_is_read 2>&1 | tail -20
```
Expected: FAIL — `no field 'role' on type 'Attestation'`.

- [ ] **Step 3: Add the `role` field**

In `game/engine/src/server_identity/attestation.rs`, add the field to the struct (after `host_hint`):

```rust
    /// What the attested key is authorised to be. `Some("player")` marks a
    /// player's per-install runtime key (online play by contact, spec §2);
    /// `None` means a server runtime key, which is the original and still the
    /// implied case. Verification does not act on this — the two consumers
    /// (`server_identity::store` and `runtime_identity`) each check the value
    /// they require, so neither can be handed the other's attestation.
    pub role: Option<String>,
```

And in `verify_structural`, next to the existing `host_hint` line:

```rust
    let host_hint = first_tag(event, "host").map(str::to_string);
    let role = first_tag(event, "role").map(str::to_string);
```

and add `role,` to the `Ok(Attestation { … })` literal.

- [ ] **Step 4: Run the test to verify it passes**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo test --bin axenstax-engine server_identity::attestation 2>&1 | tail -20
```
Expected: PASS — all attestation tests including the new one.

- [ ] **Step 5: Write the failing tests for `runtime_identity`**

Create `game/engine/src/runtime_identity.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;

    fn tmp(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("axe_rtid_{}_{}", tag, std::process::id()))
    }

    #[test]
    fn runtime_key_mints_once_and_is_stable() {
        let dir = tmp("mint");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("runtime_key.json");
        let a = load_or_mint_runtime(&path).unwrap();
        let b = load_or_mint_runtime(&path).unwrap();
        assert_eq!(a.public_key(), b.public_key(), "same key on reload");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn runtime_key_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tmp("perm");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("runtime_key.json");
        load_or_mint_runtime(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "the runtime secret is sensitive: 0600");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn mint_round_trips_through_verify_and_yields_the_persona() {
        let persona = Keys::generate();
        let runtime = Keys::generate();
        let ev = mint_player_attestation(
            &persona,
            &runtime.public_key(),
            nostr::Timestamp::from(1_000),
            PLAYER_DELEGATION_DAYS,
        )
        .await
        .unwrap();
        let got = verify_player_attestation(
            &ev,
            &runtime.public_key(),
            nostr::Timestamp::from(1_500),
        )
        .unwrap();
        assert_eq!(got, persona.public_key());
    }

    #[tokio::test]
    async fn a_server_attestation_is_refused_as_a_player_one() {
        // No `role` tag ⇒ a server attestation. It must not admit a player.
        use crate::server_identity::attestation::mint_attestation;
        let persona = Keys::generate();
        let runtime = Keys::generate();
        let ev = mint_attestation(
            &persona,
            &runtime.public_key(),
            nostr::Timestamp::from(0),
            nostr::Timestamp::from(9_999),
            &[27420],
            "",
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            verify_player_attestation(&ev, &runtime.public_key(), nostr::Timestamp::from(10)),
            Err(AttestationError::Malformed("role is not player".to_string()))
        );
    }

    #[tokio::test]
    async fn an_attestation_for_another_runtime_key_is_refused() {
        let persona = Keys::generate();
        let runtime = Keys::generate();
        let other = Keys::generate();
        let ev = mint_player_attestation(
            &persona,
            &runtime.public_key(),
            nostr::Timestamp::from(1_000),
            PLAYER_DELEGATION_DAYS,
        )
        .await
        .unwrap();
        assert_eq!(
            verify_player_attestation(&ev, &other.public_key(), nostr::Timestamp::from(1_500)),
            Err(AttestationError::WrongServerKey)
        );
    }

    #[tokio::test]
    async fn a_tampered_attestation_is_refused() {
        let persona = Keys::generate();
        let runtime = Keys::generate();
        let mut ev = mint_player_attestation(
            &persona,
            &runtime.public_key(),
            nostr::Timestamp::from(1_000),
            PLAYER_DELEGATION_DAYS,
        )
        .await
        .unwrap();
        ev.content = "tampered".to_string();
        assert_eq!(
            verify_player_attestation(&ev, &runtime.public_key(), nostr::Timestamp::from(1_500)),
            Err(AttestationError::BadSignature)
        );
    }

    #[tokio::test]
    async fn an_expired_attestation_is_refused() {
        let persona = Keys::generate();
        let runtime = Keys::generate();
        let ev = mint_player_attestation(
            &persona,
            &runtime.public_key(),
            nostr::Timestamp::from(1_000),
            1, // one day
        )
        .await
        .unwrap();
        assert_eq!(
            verify_player_attestation(
                &ev,
                &runtime.public_key(),
                nostr::Timestamp::from(1_000 + 86_400 + 1),
            ),
            Err(AttestationError::Expired)
        );
    }

    #[tokio::test]
    async fn store_then_load_round_trips_the_event() {
        let dir = tmp("store");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("runtime_attestation.json");
        let persona = Keys::generate();
        let runtime = Keys::generate();
        let ev = mint_player_attestation(
            &persona,
            &runtime.public_key(),
            nostr::Timestamp::from(1_000),
            PLAYER_DELEGATION_DAYS,
        )
        .await
        .unwrap();
        store_attestation(&path, &ev).unwrap();
        assert_eq!(load_attestation(&path).unwrap().id, ev.id);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupt_attestation_file_loads_as_absent() {
        let dir = tmp("corrupt");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("runtime_attestation.json");
        std::fs::write(&path, b"{not json").unwrap();
        assert!(load_attestation(&path).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
```

- [ ] **Step 6: Run them to verify they fail**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo test --bin axenstax-engine runtime_identity 2>&1 | tail -20
```
Expected: FAIL — `file not found for module runtime_identity` / unresolved names.

- [ ] **Step 7: Write the implementation**

Put this **above** the test module in `game/engine/src/runtime_identity.rs`:

```rust
//! The player's per-install **runtime key** and the persona attestation over it.
//!
//! The persona secret never touches this machine (it lives in a bunker). So the
//! game mints a throwaway secp256k1 key per install and asks the persona to
//! sign, exactly once, a kind-30420 attestation naming it — the same delegation
//! pattern `--pair-server` uses for a dedicated server's runtime key
//! (`server_identity::{attestation, store, pairing}`), with the tag
//! `role=player` added so the two can never be confused.
//!
//! Signalling events are signed by this runtime key; the QUIC join is still
//! signed by the persona through the bunker. The attestation travels **inside**
//! the NIP-44 ciphertext of an offer/answer and is never published, so no relay
//! can correlate a runtime key to a persona (CLAUDE.md red line 3).
//!
//! Spec: `docs/superpowers/specs/2026-09-06-online-play-by-contact-design.md` §2.
#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};

use nostr::{Event, EventBuilder, JsonUtil, Keys, Kind, NostrSigner, PublicKey, Tag, Timestamp};

use crate::server_identity::attestation::{
    verify, AttestationError, ATTESTATION_KIND, ATTESTATION_VERSION,
};

/// The `role` tag value that marks a player's runtime key.
pub const PLAYER_ROLE: &str = "player";
/// The signalling kinds a player's runtime key is authorised to sign.
pub const PLAYER_ATTESTATION_KINDS: [u16; 2] = [20900, 20901];
/// How long a player attestation is minted for. Matches the server pairing
/// default (`server_identity::pairing::DEFAULT_DELEGATION_DAYS`); re-minting is
/// one phone tap.
pub const PLAYER_DELEGATION_DAYS: u64 = 90;

/// The runtime secret. Same `profile/` tree as `signet_session.json` — kept out
/// of `worlds/` so it never shows as a world card.
pub fn runtime_key_path() -> PathBuf {
    PathBuf::from("profile").join("runtime_key.json")
}

/// The stored kind-30420 attestation event (raw JSON).
pub fn attestation_path() -> PathBuf {
    PathBuf::from("profile").join("runtime_attestation.json")
}

#[derive(serde::Serialize, serde::Deserialize)]
struct StoredKey {
    secret_hex: String,
}

/// Load the runtime key, or mint and persist one (0600) on first use.
///
/// A corrupt file re-mints, which costs exactly one bunker tap to re-attest —
/// the same degradation `native_mailbox::key::load_or_mint` accepts, and for
/// the same reason: the key addresses nothing durable on its own.
pub fn load_or_mint_runtime(path: &Path) -> Result<Keys, String> {
    if let Ok(bytes) = std::fs::read(path)
        && let Ok(stored) = serde_json::from_slice::<StoredKey>(&bytes)
        && let Ok(sk) = nostr::SecretKey::from_hex(&stored.secret_hex)
    {
        return Ok(Keys::new(sk));
    }
    let keys = Keys::generate();
    let stored = StoredKey {
        secret_hex: keys.secret_key().to_secret_hex(),
    };
    let json = serde_json::to_vec(&stored).map_err(|e| format!("serialise runtime key: {e}"))?;
    write_0600(path, &json)?;
    Ok(keys)
}

/// Write owner-only, creating the parent directory. Mirrors
/// `server_identity::store::write_0600` (which is `pub(crate)` to that module).
fn write_0600(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .map_err(|e| format!("open {}: {e}", path.display()))?;
        return f.write_all(bytes).map_err(|e| format!("write: {e}"));
    }
    #[cfg(not(unix))]
    std::fs::write(path, bytes).map_err(|e| format!("write: {e}"))
}

/// Read the stored attestation. Any absence or corruption reads as `None` — the
/// player is simply not yet attested, which the UI explains and a bunker tap
/// fixes.
pub fn load_attestation(path: &Path) -> Option<Event> {
    let s = std::fs::read_to_string(path).ok()?;
    Event::from_json(&s).ok()
}

/// Persist an attestation event verbatim (0600 — it names the persona, which is
/// not secret, but it sits beside the key and there is no reason to widen it).
pub fn store_attestation(path: &Path, ev: &Event) -> Result<(), String> {
    write_0600(path, ev.as_json().as_bytes())
}

/// Ask the persona (a bunker in production, a local `Keys` in tests — both
/// implement `NostrSigner`) to attest `runtime`. Producing the signature is the
/// only online step: for a bunker it is one tap on the player's phone.
pub async fn mint_player_attestation<S: NostrSigner>(
    persona: &S,
    runtime: &PublicKey,
    now: Timestamp,
    validity_days: u64,
) -> Result<Event, String> {
    let until = Timestamp::from(now.as_secs() + validity_days.saturating_mul(86_400));
    let mut tags = vec![
        Tag::parse(["d", &runtime.to_hex()]).map_err(|e| e.to_string())?,
        Tag::parse(["p", &runtime.to_hex()]).map_err(|e| e.to_string())?,
        Tag::parse(["valid_from", &now.as_secs().to_string()]).map_err(|e| e.to_string())?,
        Tag::parse(["valid_until", &until.as_secs().to_string()]).map_err(|e| e.to_string())?,
        Tag::parse(["v", ATTESTATION_VERSION]).map_err(|e| e.to_string())?,
        // A player attestation names no server, so `name` is empty and there is
        // no `host` tag. `verify_structural` tolerates both.
        Tag::parse(["name", ""]).map_err(|e| e.to_string())?,
        Tag::parse(["role", PLAYER_ROLE]).map_err(|e| e.to_string())?,
    ];
    for k in PLAYER_ATTESTATION_KINDS {
        tags.push(Tag::parse(["k", &k.to_string()]).map_err(|e| e.to_string())?);
    }
    EventBuilder::new(Kind::Custom(ATTESTATION_KIND), "")
        .tags(tags)
        .sign(persona)
        .await
        .map_err(|e| e.to_string())
}

/// Verify a player attestation and return the attesting persona.
///
/// Everything `server_identity::attestation::verify` checks (signature, kind,
/// version, `d` == `expected_runtime`, validity window) plus the `role=player`
/// requirement — which is what stops a server's delegation being replayed as a
/// player's, and vice versa.
pub fn verify_player_attestation(
    ev: &Event,
    expected_runtime: &PublicKey,
    now: Timestamp,
) -> Result<PublicKey, AttestationError> {
    let att = verify(ev, expected_runtime, now)?;
    if att.role.as_deref() != Some(PLAYER_ROLE) {
        return Err(AttestationError::Malformed("role is not player".to_string()));
    }
    Ok(att.operator)
}

/// This install's online-play identity: the runtime key plus, once the player
/// has tapped their phone, the persona's attestation over it.
pub struct RuntimeIdentity {
    keys: Keys,
    attestation: Option<Event>,
}

impl RuntimeIdentity {
    /// Load (minting the runtime key on first use). `Ok(None)` is never
    /// returned today — it is the shape a future "no profile dir" case would
    /// take; an unattested identity is `Some` with `attestation() == None`.
    pub fn load() -> Result<Option<RuntimeIdentity>, String> {
        let keys = load_or_mint_runtime(&runtime_key_path())?;
        let attestation = load_attestation(&attestation_path());
        Ok(Some(RuntimeIdentity { keys, attestation }))
    }

    pub fn keys(&self) -> &Keys {
        &self.keys
    }

    pub fn runtime_pubkey(&self) -> PublicKey {
        self.keys.public_key()
    }

    pub fn attestation(&self) -> Option<&Event> {
        self.attestation.as_ref()
    }

    /// The persona this runtime key is currently attested by, if the
    /// attestation is present and valid at `now`.
    pub fn persona(&self, now: Timestamp) -> Option<PublicKey> {
        let ev = self.attestation.as_ref()?;
        verify_player_attestation(ev, &self.keys.public_key(), now).ok()
    }
}
```

- [ ] **Step 8: Register the module**

In `game/engine/src/main.rs`, next to the `mod invite;` added in Task 1:

```rust
// Online play by contact — the per-install runtime key + its persona
// attestation. Native only (bunker signing, profile/ files).
#[cfg(not(target_arch = "wasm32"))]
mod runtime_identity;
```

- [ ] **Step 9: Run the tests to verify they pass**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo test --bin axenstax-engine runtime_identity 2>&1 | tail -20
```
Expected: PASS — 8 passed.

- [ ] **Step 10: Verify clippy and the wasm build are clean**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo clippy -- -D warnings 2>&1 | tail -20
```
Expected: no warnings. Then:
```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 trunk build 2>&1 | tail -10
```
Expected: `success` — the new module must not have leaked into the wasm graph.

- [ ] **Step 11: Commit**

```bash
git add game/engine/src/runtime_identity.rs game/engine/src/server_identity/attestation.rs game/engine/src/main.rs && git commit -m "$(cat <<'EOF'
feat(online): runtime key + role=player attestation (P1)

Per-install secp256k1 runtime key (profile/runtime_key.json, 0600) attested
once by the player's Signet persona as a kind-30420 event carrying role=player.
verify_structural now reads the role tag; absent still means server, so a
server delegation can never be replayed as a player's and vice versa.

Claude-Session: https://claude.ai/code/session_011gW6q5QFBTbF1BNMHiayce
EOF
)" -- game/engine/src/runtime_identity.rs game/engine/src/server_identity/attestation.rs game/engine/src/main.rs
```

---

### Task 3: The contacts mirror (`contacts.rs` extension)

**Files:**
- Modify: `game/engine/src/comms.rs` (serde on `Tier`)
- Modify: `game/engine/src/contacts.rs` (`Contact` fields, mirror file, union)

**Interfaces:**
- Consumes:
  ```rust
  // src/comms.rs — unchanged variants, DO NOT reorder
  pub enum Tier { Kin, Kith, Ken, Stranger }
  // src/contacts.rs — existing
  pub struct Contact { pub pubkey: [u8; 32], pub display_name: Option<String>, pub tier: Tier, pub is_child: bool }
  pub fn parse_kenspeckle_export(blob: &[u8], key: &[u8; 32]) -> Result<Vec<Contact>, ContactsParseError>;
  pub fn load_local_book() -> Vec<Contact>;   // signature UNCHANGED — hosted_server.rs:~390 calls it
  ```
- Produces:
  ```rust
  #[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
  #[serde(rename_all = "lowercase")]
  pub enum AddedVia { Invite, Import, Paste, Kenspeckle }

  pub struct Contact {
      pub pubkey: [u8; 32],
      pub display_name: Option<String>,
      pub tier: Tier,
      pub is_child: bool,
      pub runtime_pubkey: Option<[u8; 32]>,
      pub added_via: AddedVia,
      pub added_at: u64,
  }
  pub const MIRROR_VERSION: u32 = 1;
  pub fn mirror_path() -> std::path::PathBuf;                                  // profile/contacts.json
  pub fn load_mirror(path: &Path) -> Vec<Contact>;
  pub fn save_mirror(path: &Path, contacts: &[Contact]) -> Result<(), String>;
  pub fn upsert(book: &mut Vec<Contact>, incoming: Contact);
  pub fn merge_books(kenspeckle: Vec<Contact>, mirror: Vec<Contact>) -> Vec<Contact>;
  pub fn find<'a>(book: &'a [Contact], pubkey: &[u8; 32]) -> Option<&'a Contact>;
  pub fn tier_of(book: &[Contact], pubkey: &[u8; 32]) -> Tier;
  ```

- [ ] **Step 1: Write the failing tests**

Append to the `mod tests` block at the bottom of `game/engine/src/contacts.rs`:

```rust
    // ─── The mirror (online play by contact, spec §2/§6) ───

    fn c(pk: u8, tier: Tier, via: AddedVia, at: u64) -> Contact {
        Contact {
            pubkey: [pk; 32],
            display_name: Some(format!("P{pk}")),
            tier,
            is_child: false,
            runtime_pubkey: None,
            added_via: via,
            added_at: at,
        }
    }

    #[test]
    fn mirror_round_trips_through_the_file() {
        let dir = std::env::temp_dir().join(format!("axe_mirror_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("contacts.json");
        let book = vec![
            Contact {
                runtime_pubkey: Some([0x5a; 32]),
                ..c(1, Tier::Kith, AddedVia::Invite, 1_700_000_000)
            },
            c(2, Tier::Kin, AddedVia::Paste, 1_700_000_001),
        ];
        save_mirror(&path, &book).unwrap();
        assert_eq!(load_mirror(&path), book);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_or_corrupt_mirror_reads_as_empty() {
        let dir = std::env::temp_dir().join(format!("axe_mirror_bad_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(load_mirror(&dir.join("contacts.json")).is_empty());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("contacts.json"), b"{not json").unwrap();
        assert!(load_mirror(&dir.join("contacts.json")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn upsert_inserts_then_updates_without_duplicating() {
        let mut book = vec![c(1, Tier::Ken, AddedVia::Kenspeckle, 100)];
        upsert(&mut book, c(2, Tier::Kith, AddedVia::Invite, 200));
        assert_eq!(book.len(), 2);
        // Re-adding #1 at a closer tier upgrades it and records the runtime key,
        // but keeps the ORIGINAL added_at (when you first met them).
        upsert(
            &mut book,
            Contact {
                runtime_pubkey: Some([9u8; 32]),
                ..c(1, Tier::Kith, AddedVia::Invite, 300)
            },
        );
        assert_eq!(book.len(), 2, "same pubkey must not duplicate");
        let one = find(&book, &[1u8; 32]).unwrap();
        assert_eq!(one.tier, Tier::Kith);
        assert_eq!(one.runtime_pubkey, Some([9u8; 32]));
        assert_eq!(one.added_at, 100, "added_at is when you first met them");
    }

    #[test]
    fn upsert_never_loosens_a_tier() {
        // Being handed an invite must not demote somebody you already call Kin.
        let mut book = vec![c(1, Tier::Kin, AddedVia::Kenspeckle, 100)];
        upsert(&mut book, c(1, Tier::Ken, AddedVia::Invite, 200));
        assert_eq!(find(&book, &[1u8; 32]).unwrap().tier, Tier::Kin);
    }

    #[test]
    fn merge_prefers_the_mirror_for_a_pubkey_in_both() {
        let kenspeckle = vec![c(1, Tier::Ken, AddedVia::Kenspeckle, 0)];
        let mirror = vec![Contact {
            runtime_pubkey: Some([3u8; 32]),
            ..c(1, Tier::Kith, AddedVia::Invite, 500)
        }];
        let merged = merge_books(kenspeckle, mirror);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].tier, Tier::Kith, "the closer tier wins");
        assert_eq!(merged[0].runtime_pubkey, Some([3u8; 32]));
    }

    #[test]
    fn merge_is_the_union_of_both_books() {
        let merged = merge_books(
            vec![c(1, Tier::Kin, AddedVia::Kenspeckle, 0)],
            vec![c(2, Tier::Kith, AddedVia::Invite, 1)],
        );
        assert_eq!(merged.len(), 2);
        assert!(find(&merged, &[1u8; 32]).is_some());
        assert!(find(&merged, &[2u8; 32]).is_some());
    }

    #[test]
    fn tier_of_falls_back_to_stranger() {
        let book = vec![c(1, Tier::Kin, AddedVia::Kenspeckle, 0)];
        assert_eq!(tier_of(&book, &[1u8; 32]), Tier::Kin);
        assert_eq!(tier_of(&book, &[9u8; 32]), Tier::Stranger);
    }

    #[test]
    fn kenspeckle_entries_are_tagged_as_such() {
        let blob = include_bytes!("../assets/test/kenspeckle-export.v1.bin");
        let contacts = parse_kenspeckle_export(blob, &fixture_key()).unwrap();
        assert!(
            contacts.iter().all(|c| c.added_via == AddedVia::Kenspeckle),
            "an imported entry must say where it came from"
        );
        assert!(
            contacts.iter().all(|c| c.runtime_pubkey.is_none()),
            "Kenspeckle carries no runtime key — only a rendezvous can supply one"
        );
    }
```

- [ ] **Step 2: Run them to verify they fail**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo test --bin axenstax-engine contacts:: 2>&1 | tail -20
```
Expected: FAIL — `cannot find type AddedVia`, `struct Contact has no field named runtime_pubkey`.

- [ ] **Step 3: Add serde to `Tier`**

In `game/engine/src/comms.rs`, change the `Tier` derive line (currently `#[derive(Clone, Copy, Debug, PartialEq, Eq)]`, immediately above `pub enum Tier {`) to:

```rust
// `serde` so the contacts mirror (`contacts::save_mirror`, online play by
// contact §6) can round-trip a tier as `"kin" | "kith" | "ken" | "stranger"`.
// Deliberately NOT `Ord`: "close enough to play in my world" is
// `matches!(t, Tier::Kin | Tier::Kith)` in `online_admission`, an explicit
// enumeration, not a comparison that a variant reorder could silently change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
```

- [ ] **Step 4: Extend `Contact` and add the mirror**

In `game/engine/src/contacts.rs`:

(a) Replace the `Contact` struct with:

```rust
/// Where a contact came into the book from. Advisory (it drives UI copy and
/// lets a future "forget everyone I met by invite" do the right thing); it is
/// never a permission.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AddedVia {
    /// Admitted by, or admitted with, an invite bearer.
    Invite,
    /// Imported from a Signet persona-scoped contacts view (waits upstream).
    Import,
    /// The player pasted an npub.
    Paste,
    /// Came out of a Kenspeckle export.
    Kenspeckle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Contact {
    /// x-only pubkey, hex on the wire, npub for display.
    pub pubkey: [u8; 32],
    /// Absent for a `ken` entry with no display name in the source (the null
    /// case the frozen fixture exists to cover).
    pub display_name: Option<String>,
    pub tier: Tier,
    /// Advisory only — drives UI copy, never a permission. See the doc
    /// comment on `convert_entry` for how (and why only how) this is derived.
    pub is_child: bool,
    /// The contact's per-install signalling key, learned from a verified
    /// rendezvous (online play by contact §2). `None` until they have called
    /// or answered once. Kenspeckle never carries it.
    pub runtime_pubkey: Option<[u8; 32]>,
    pub added_via: AddedVia,
    /// Unix seconds when this contact first entered the book. Preserved across
    /// upserts — it is "when you met", not "when the row was last touched".
    pub added_at: u64,
}
```

(b) In `convert_entry`, extend the returned literal:

```rust
    Ok(Contact {
        pubkey,
        display_name: e.display_name,
        tier,
        is_child,
        // Kenspeckle carries neither of these: a runtime key can only come from
        // a verified rendezvous, and an export has no per-entry timestamp we
        // keep (§ the stripping boundary above).
        runtime_pubkey: None,
        added_via: AddedVia::Kenspeckle,
        added_at: 0,
    })
```

(c) Append the mirror section, immediately **before** the `// ─── The QR chunk transport` banner:

```rust
// ─── The local mirror (online play by contact, spec §2/§6) ──────────────────
//
// `profile/contacts.json` — the player's own address book, fed by invites and
// pastes today and by the Signet persona-scoped contacts view when that ships
// upstream. It is a LOCAL FILE and nothing else: no directory, no sync, no
// server copy (CLAUDE.md red lines 1 and 3).

/// On-disk format version. Fields are append-only: an older build reading a
/// newer file must degrade, never refuse.
pub const MIRROR_VERSION: u32 = 1;

/// `profile/contacts.json` — the same `profile/` tree as the runtime key and
/// the Signet session.
pub fn mirror_path() -> std::path::PathBuf {
    std::path::PathBuf::from("profile").join("contacts.json")
}

#[derive(serde::Serialize, serde::Deserialize)]
struct MirrorFile {
    v: u32,
    contacts: Vec<StoredContact>,
}

/// The persisted shape. Pubkeys are hex here and npub only at the display
/// boundary ([[feedback_npub_only_display]] governs what a *person* sees, not
/// what a file holds).
#[derive(serde::Serialize, serde::Deserialize)]
struct StoredContact {
    pubkey: String,
    #[serde(default)]
    display_name: Option<String>,
    tier: Tier,
    #[serde(default)]
    is_child: bool,
    #[serde(default)]
    runtime_pubkey: Option<String>,
    added_via: AddedVia,
    #[serde(default)]
    added_at: u64,
}

/// Read the mirror. Any absence, corruption, or unreadable entry yields an
/// empty book — safe by construction, because an empty book means `Stranger`
/// for everyone and therefore admits nobody.
pub fn load_mirror(path: &std::path::Path) -> Vec<Contact> {
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    let Ok(file) = serde_json::from_slice::<MirrorFile>(&bytes) else {
        return Vec::new();
    };
    file.contacts
        .into_iter()
        .filter_map(|s| {
            let pubkey = hex::decode(&s.pubkey)
                .ok()
                .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())?;
            let runtime_pubkey = match s.runtime_pubkey {
                Some(h) => Some(
                    hex::decode(&h)
                        .ok()
                        .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())?,
                ),
                None => None,
            };
            Some(Contact {
                pubkey,
                display_name: s.display_name,
                tier: s.tier,
                is_child: s.is_child,
                runtime_pubkey,
                added_via: s.added_via,
                added_at: s.added_at,
            })
        })
        .collect()
}

/// Write the mirror (0600 on unix — it is a list of who a child knows).
pub fn save_mirror(path: &std::path::Path, contacts: &[Contact]) -> Result<(), String> {
    let file = MirrorFile {
        v: MIRROR_VERSION,
        contacts: contacts
            .iter()
            .map(|c| StoredContact {
                pubkey: hex::encode(c.pubkey),
                display_name: c.display_name.clone(),
                tier: c.tier,
                is_child: c.is_child,
                runtime_pubkey: c.runtime_pubkey.map(hex::encode),
                added_via: c.added_via,
                added_at: c.added_at,
            })
            .collect(),
    };
    let json = serde_json::to_vec_pretty(&file).map_err(|e| format!("serialise contacts: {e}"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    std::fs::write(path, json).map_err(|e| format!("write contacts: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// Closeness rank, low = closer. Local to this module so `Tier` itself stays
/// un-`Ord` (see the derive comment in `comms.rs`).
fn closeness(t: Tier) -> u8 {
    match t {
        Tier::Kin => 0,
        Tier::Kith => 1,
        Tier::Ken => 2,
        Tier::Stranger => 3,
    }
}

/// Insert `incoming`, or fold it into the existing row for the same pubkey.
///
/// Merge rule, and why: the **closer** tier wins (an invite must never demote
/// somebody you already call Kin), the **earliest** `added_at` wins (it records
/// when you met, not when the row was last touched), a newly-learned
/// `runtime_pubkey` and display name overwrite, and `added_via` records the
/// most recent route in.
pub fn upsert(book: &mut Vec<Contact>, incoming: Contact) {
    if let Some(existing) = book.iter_mut().find(|c| c.pubkey == incoming.pubkey) {
        if closeness(incoming.tier) < closeness(existing.tier) {
            existing.tier = incoming.tier;
        }
        if incoming.runtime_pubkey.is_some() {
            existing.runtime_pubkey = incoming.runtime_pubkey;
        }
        if incoming.display_name.is_some() {
            existing.display_name = incoming.display_name;
        }
        existing.is_child |= incoming.is_child;
        existing.added_at = existing.added_at.min(incoming.added_at);
        existing.added_via = incoming.added_via;
    } else {
        book.push(incoming);
    }
}

/// The union of the Kenspeckle export and the local mirror, with the mirror
/// folded in on top (so a tier the player set themselves wins over an import).
pub fn merge_books(kenspeckle: Vec<Contact>, mirror: Vec<Contact>) -> Vec<Contact> {
    let mut out = kenspeckle;
    for c in mirror {
        upsert(&mut out, c);
    }
    out
}

/// Look a contact up by pubkey.
pub fn find<'a>(book: &'a [Contact], pubkey: &[u8; 32]) -> Option<&'a Contact> {
    book.iter().find(|c| &c.pubkey == pubkey)
}

/// The tier a book records for `pubkey`, falling back to `Stranger` — the same
/// fail-closed default `ServerPlayer::tier_of` uses.
pub fn tier_of(book: &[Contact], pubkey: &[u8; 32]) -> Tier {
    find(book, pubkey).map(|c| c.tier).unwrap_or(Tier::Stranger)
}
```

(d) Change `load_local_book` to return the union. Replace its final line
`parse_kenspeckle_export(&blob, &key).unwrap_or_default()` with:

```rust
    let imported = parse_kenspeckle_export(&blob, &key).unwrap_or_default();
    merge_books(imported, load_mirror(&mirror_path()))
```

and add an early-return guard so the mirror is still read when there is no
Kenspeckle export at all — replace each of the four `return Vec::new();`
early-returns in `load_local_book` with `return load_mirror(&mirror_path());`,
and update its doc comment's first line to:

```rust
/// Best-effort local address book for the local player: the union of the
/// Kenspeckle export (if a guardian has dropped one in the config dir) and the
/// player's own `profile/contacts.json` mirror (online play by contact §6).
```

(e) Update `to_persisted_json` so the stripping test still compiles — it builds
its JSON from the four original fields, which is still correct; add the three
new ones so the test keeps proving what it claims:

```rust
                "runtime_pubkey": c.runtime_pubkey.map(hex::encode),
                "added_via": match c.added_via {
                    AddedVia::Invite => "invite",
                    AddedVia::Import => "import",
                    AddedVia::Paste => "paste",
                    AddedVia::Kenspeckle => "kenspeckle",
                },
                "added_at": c.added_at,
```

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo test --bin axenstax-engine contacts:: 2>&1 | tail -20
```
Expected: PASS — the 8 new tests plus the 11 pre-existing ones.

- [ ] **Step 6: Verify the whole suite and clippy still pass**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo clippy -- -D warnings 2>&1 | tail -20
```
Expected: no warnings. `hosted_server.rs` builds `sp.contacts` from `load_local_book()` with `.map(|c| (c.pubkey, c.tier))` — unchanged and still compiles, now fed by the union.

- [ ] **Step 7: Commit**

```bash
git add game/engine/src/comms.rs game/engine/src/contacts.rs && git commit -m "$(cat <<'EOF'
feat(online): contacts mirror + Contact gains runtime key/provenance (P1)

profile/contacts.json is the player's own address book, fed by invites and
pastes now and by Signet's persona-scoped contacts view when it ships.
load_local_book() is now the union of the Kenspeckle export and the mirror;
upsert never loosens a tier and keeps the earliest added_at. Tier gains serde
(lowercase) but deliberately NOT Ord.

Claude-Session: https://claude.ai/code/session_011gW6q5QFBTbF1BNMHiayce
EOF
)" -- game/engine/src/comms.rs game/engine/src/contacts.rs
```

---

### Task 4: The admission rule (`online_admission.rs`)

**Files:**
- Create: `game/engine/src/online_admission.rs`
- Modify: `game/engine/src/main.rs`

**Interfaces:**
- Consumes: `crate::comms::Tier`, `crate::contacts::{Contact, tier_of}`, `crate::invite::BEARER_LEN`.
- Produces:
  ```rust
  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  pub enum AdmitReason { AlreadyContact, ByInvite }
  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  pub enum Refusal { NotAContact, BearerInvalid, BearerExpired, ProtocolMismatch, Full, HostBusy }
  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  pub enum Admission { Accept(AdmitReason), Refuse(Refusal) }
  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  pub struct ActiveBearer { pub bearer: [u8; 16], pub expires_at: u64 }
  pub fn admits_play(tier: crate::comms::Tier) -> bool;
  pub fn admit(persona: &[u8; 32], offered: Option<[u8; 16]>, book: &[Contact],
               active: Option<&ActiveBearer>, now: u64) -> Admission;
  pub fn is_reply_worthy(r: Refusal) -> bool;
  pub fn refusal_wire(r: Refusal) -> &'static str;
  pub fn refusal_from_wire(s: &str) -> Option<Refusal>;
  ```

- [ ] **Step 1: Write the failing tests**

Create `game/engine/src/online_admission.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::comms::{Tier, ALL_TIERS};
    use crate::contacts::{AddedVia, Contact};

    const ALICE: [u8; 32] = [1u8; 32];
    const BEARER: [u8; 16] = [7u8; 16];
    const WRONG: [u8; 16] = [8u8; 16];

    fn book_at(tier: Tier) -> Vec<Contact> {
        vec![Contact {
            pubkey: ALICE,
            display_name: Some("Alice".to_string()),
            tier,
            is_child: false,
            runtime_pubkey: None,
            added_via: AddedVia::Kenspeckle,
            added_at: 0,
        }]
    }

    fn active() -> ActiveBearer {
        ActiveBearer { bearer: BEARER, expires_at: 1_000 }
    }

    #[test]
    fn every_tier_is_decided_and_only_kin_and_kith_play() {
        // Exhaustive over the tier axis, so adding a tier later fails here
        // rather than silently defaulting somebody in.
        for tier in ALL_TIERS {
            let got = admit(&ALICE, None, &book_at(tier), Some(&active()), 500);
            match tier {
                Tier::Kin | Tier::Kith => {
                    assert_eq!(got, Admission::Accept(AdmitReason::AlreadyContact), "{tier:?}")
                }
                // Ken is hear-only in comms; it is not "play in my world".
                Tier::Ken | Tier::Stranger => {
                    assert_eq!(got, Admission::Refuse(Refusal::NotAContact), "{tier:?}")
                }
            }
        }
    }

    #[test]
    fn a_stranger_with_the_live_bearer_is_admitted() {
        assert_eq!(
            admit(&ALICE, Some(BEARER), &[], Some(&active()), 500),
            Admission::Accept(AdmitReason::ByInvite)
        );
    }

    #[test]
    fn the_bearer_is_valid_up_to_and_including_its_expiry_second() {
        assert_eq!(
            admit(&ALICE, Some(BEARER), &[], Some(&active()), 1_000),
            Admission::Accept(AdmitReason::ByInvite)
        );
        assert_eq!(
            admit(&ALICE, Some(BEARER), &[], Some(&active()), 1_001),
            Admission::Refuse(Refusal::BearerExpired)
        );
    }

    #[test]
    fn a_wrong_bearer_is_invalid_not_expired() {
        assert_eq!(
            admit(&ALICE, Some(WRONG), &[], Some(&active()), 500),
            Admission::Refuse(Refusal::BearerInvalid)
        );
    }

    #[test]
    fn a_bearer_with_no_invite_open_is_invalid() {
        assert_eq!(
            admit(&ALICE, Some(BEARER), &[], None, 500),
            Admission::Refuse(Refusal::BearerInvalid)
        );
    }

    #[test]
    fn a_stranger_with_no_bearer_is_simply_not_a_contact() {
        assert_eq!(
            admit(&ALICE, None, &[], Some(&active()), 500),
            Admission::Refuse(Refusal::NotAContact)
        );
    }

    #[test]
    fn being_a_contact_beats_a_wrong_bearer() {
        // A Kith friend who pasted a stale link still gets in.
        assert_eq!(
            admit(&ALICE, Some(WRONG), &book_at(Tier::Kith), Some(&active()), 500),
            Admission::Accept(AdmitReason::AlreadyContact)
        );
    }

    #[test]
    fn only_protocol_mismatch_and_full_are_ever_answered() {
        // The host stays SILENT to strangers — an answer would confirm it is
        // there and hosting. Only the two refusals that are useful to a person
        // who is already entitled to know get sent.
        assert!(is_reply_worthy(Refusal::ProtocolMismatch));
        assert!(is_reply_worthy(Refusal::Full));
        for r in [
            Refusal::NotAContact,
            Refusal::BearerInvalid,
            Refusal::BearerExpired,
            Refusal::HostBusy,
        ] {
            assert!(!is_reply_worthy(r), "{r:?} must never be sent");
        }
    }

    #[test]
    fn refusal_wire_round_trips() {
        for r in [
            Refusal::NotAContact,
            Refusal::BearerInvalid,
            Refusal::BearerExpired,
            Refusal::ProtocolMismatch,
            Refusal::Full,
            Refusal::HostBusy,
        ] {
            assert_eq!(refusal_from_wire(refusal_wire(r)), Some(r));
        }
        assert_eq!(refusal_from_wire("something-else"), None);
    }
}
```

- [ ] **Step 2: Run them to verify they fail**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo test --bin axenstax-engine online_admission 2>&1 | tail -20
```
Expected: FAIL — `file not found for module online_admission`.

- [ ] **Step 3: Write the implementation**

Put this above the test module in `game/engine/src/online_admission.rs`:

```rust
//! Who may join an online-hosted world — the whole rule, in one pure function.
//!
//! Named `online_admission` because `crate::admission` already exists and means
//! something else entirely (capacity: is the server full). This module is about
//! *identity*: is this person somebody the host actually knows, or somebody
//! holding an invite the host just minted.
//!
//! The rule (spec §3.3):
//!
//! ```text
//! admit(offer, contacts, active_bearer, now) =
//!     if tier_of(persona) is Kin or Kith        -> Accept(AlreadyContact)
//!     else if bearer matches and is unexpired   -> Accept(ByInvite)
//!     else                                      -> Refuse(...)
//! ```
//!
//! **Ken does not admit.** Ken is one-way recognition — I pinned you, you did
//! not pin me — and `comms.rs` already treats it as hear-only. Pinning somebody
//! must not hand them a key to your house either.
//!
//! Refusals are mostly **never sent**. A stranger gets silence, because an
//! answer is itself information ("yes, somebody is hosting here"). Only
//! `ProtocolMismatch` and `Full` go on the wire, and only to somebody who was
//! already entitled to reach the host.
#![cfg(not(target_arch = "wasm32"))]

use crate::comms::Tier;
use crate::contacts::{tier_of, Contact};
use crate::invite::BEARER_LEN;

/// Why a join was admitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdmitReason {
    /// Already in the host's book at Kin or Kith.
    AlreadyContact,
    /// Presented the session's live bearer. The caller adds them as a contact.
    ByInvite,
}

/// Why a join was refused. `ProtocolMismatch` and `Full` are decided by the
/// caller (they are not identity questions) but live here so one enum covers
/// every refusal that can appear in an `Answer`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    NotAContact,
    BearerInvalid,
    BearerExpired,
    ProtocolMismatch,
    Full,
    HostBusy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admission {
    Accept(AdmitReason),
    Refuse(Refusal),
}

/// The invite a host currently has open. Minting a fresh invite replaces this,
/// which is what "retires the old bearer" means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActiveBearer {
    pub bearer: [u8; BEARER_LEN],
    pub expires_at: u64,
}

/// Whether a tier is close enough to play in someone's world.
///
/// An explicit enumeration rather than `tier <= Kith`: `Tier` is deliberately
/// not `Ord` (see the derive comment in `comms.rs`), so reordering the variants
/// cannot silently widen this.
pub fn admits_play(tier: Tier) -> bool {
    matches!(tier, Tier::Kin | Tier::Kith)
}

/// Constant-time 16-byte comparison. A bearer is a shared secret, and the
/// number of relay round-trips an attacker can drive is not something this
/// process controls, so the comparison does not short-circuit on the first
/// differing byte. No new dependency — it is four lines.
fn bearer_eq(a: &[u8; BEARER_LEN], b: &[u8; BEARER_LEN]) -> bool {
    let mut diff = 0u8;
    for i in 0..BEARER_LEN {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

/// The admission decision. Pure: `now` is unix seconds, injected.
pub fn admit(
    persona: &[u8; 32],
    offered: Option<[u8; BEARER_LEN]>,
    book: &[Contact],
    active: Option<&ActiveBearer>,
    now: u64,
) -> Admission {
    if admits_play(tier_of(book, persona)) {
        return Admission::Accept(AdmitReason::AlreadyContact);
    }
    match (offered, active) {
        (Some(offered), Some(active)) if bearer_eq(&offered, &active.bearer) => {
            if now <= active.expires_at {
                Admission::Accept(AdmitReason::ByInvite)
            } else {
                Admission::Refuse(Refusal::BearerExpired)
            }
        }
        // A bearer was presented but it isn't the live one (or there is no live
        // one at all).
        (Some(_), _) => Admission::Refuse(Refusal::BearerInvalid),
        (None, _) => Admission::Refuse(Refusal::NotAContact),
    }
}

/// Whether a refusal may be put on the wire at all.
///
/// Silence is the default. `NotAContact` and the bearer refusals would tell a
/// stranger that somebody is hosting here and that their guess was close, so
/// they are never sent. `ProtocolMismatch` and `Full` are actionable facts for
/// somebody who already got through the door once.
pub fn is_reply_worthy(r: Refusal) -> bool {
    matches!(r, Refusal::ProtocolMismatch | Refusal::Full)
}

/// The wire spelling of a refusal, for the `Answer.reason` field.
pub fn refusal_wire(r: Refusal) -> &'static str {
    match r {
        Refusal::NotAContact => "not-a-contact",
        Refusal::BearerInvalid => "bearer-invalid",
        Refusal::BearerExpired => "bearer-expired",
        Refusal::ProtocolMismatch => "protocol-mismatch",
        Refusal::Full => "full",
        Refusal::HostBusy => "host-busy",
    }
}

/// Inverse of [`refusal_wire`]. Unknown text yields `None` rather than a
/// default, so a future refusal kind reads as "unexplained" and not as "full".
pub fn refusal_from_wire(s: &str) -> Option<Refusal> {
    match s {
        "not-a-contact" => Some(Refusal::NotAContact),
        "bearer-invalid" => Some(Refusal::BearerInvalid),
        "bearer-expired" => Some(Refusal::BearerExpired),
        "protocol-mismatch" => Some(Refusal::ProtocolMismatch),
        "full" => Some(Refusal::Full),
        "host-busy" => Some(Refusal::HostBusy),
        _ => None,
    }
}
```

- [ ] **Step 4: Register the module**

In `game/engine/src/main.rs`, next to `mod admission;` (line ~118):

```rust
// Online play by contact — WHO may join (identity), as distinct from `admission`
// above, which is WHETHER there is room (capacity). Native only.
#[cfg(not(target_arch = "wasm32"))]
mod online_admission;
```

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo test --bin axenstax-engine online_admission 2>&1 | tail -20
```
Expected: PASS — 9 passed.

- [ ] **Step 6: Commit**

```bash
git add game/engine/src/online_admission.rs game/engine/src/main.rs && git commit -m "$(cat <<'EOF'
feat(online): the admission rule — contacts at Kin/Kith, or a live bearer (P1)

Pure decision, exhaustive over the tier axis. Ken deliberately does NOT admit:
one-way recognition is hear-only in comms and is not a key to the house.
Strangers get silence — only ProtocolMismatch and Full are ever answered.
Named online_admission because admission.rs already means capacity.

Claude-Session: https://claude.ai/code/session_011gW6q5QFBTbF1BNMHiayce
EOF
)" -- game/engine/src/online_admission.rs game/engine/src/main.rs
```

---

### Task 5: "Your address" panel (`friends_ui.rs`)

**Files:**
- Create: `game/engine/src/friends_ui.rs`
- Modify: `game/engine/src/main.rs`
- Modify: `game/engine/src/menu.rs` (make `draw_qr` reachable; call the panel from the servers column)

**Interfaces:**
- Consumes:
  ```rust
  // src/menu.rs — currently private, becomes pub(crate)
  fn draw_qr(ui: &mut egui::Ui, data: &str, size_px: f32);
  // src/signet/native_signer.rs
  pub fn load_identity() -> NativeIdentity;
  impl NativeIdentity { pub fn npub(&self) -> Option<String>; pub fn is_signed_in(&self) -> bool; }
  ```
- Produces:
  ```rust
  pub fn your_address_lead() -> &'static str;
  pub fn short_npub(npub: &str) -> String;
  pub fn draw_your_address(ui: &mut egui::Ui, npub: Option<&str>);
  ```

- [ ] **Step 1: Write the failing tests**

Create `game/engine/src/friends_ui.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const NPUB: &str = "npub1sg6plzptd64u62a878hep2kev88swjh3tw00gjsfl8f237lmu63q0uf63m";

    #[test]
    fn the_lead_line_is_the_approved_copy() {
        // Pinned so the wording can't drift: it must explain what the address is
        // FOR (being invited), not invite anyone to find anyone. No discovery
        // language, no social-network framing (CLAUDE.md red line 4).
        assert_eq!(
            your_address_lead(),
            "Give this to a friend so they can invite you."
        );
    }

    #[test]
    fn the_lead_line_carries_no_discovery_or_money_words() {
        let lead = your_address_lead().to_lowercase();
        for banned in [
            "browse", "discover", "find players", "directory", "public",
            "earn", "sats", "bitcoin", "money", "social network",
        ] {
            assert!(!lead.contains(banned), "lead copy must not say {banned:?}");
        }
    }

    #[test]
    fn short_npub_keeps_the_npub_prefix_and_the_tail() {
        let s = short_npub(NPUB);
        assert!(s.starts_with("npub1"), "must still read as an npub: {s}");
        assert!(s.ends_with(&NPUB[NPUB.len() - 4..]), "tail preserved: {s}");
        assert!(s.contains('…'));
        assert!(s.len() < NPUB.len());
    }

    #[test]
    fn short_npub_leaves_a_short_string_alone() {
        assert_eq!(short_npub("npub1abc"), "npub1abc");
    }
}
```

- [ ] **Step 2: Run them to verify they fail**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo test --bin axenstax-engine friends_ui 2>&1 | tail -20
```
Expected: FAIL — `file not found for module friends_ui`.

- [ ] **Step 3: Write the implementation**

Put this above the test module in `game/engine/src/friends_ui.rs`:

```rust
//! The lobby's people surface: your own address, and (Task 17) the "Friends &
//! servers" column.
//!
//! This is **not** a social surface. It shows people you already know, so you
//! can build with them. There is no browsing, no discovery, and nothing here
//! lists a world (CLAUDE.md red lines 1 and 4). Lives in its own file because
//! `menu.rs` is already 6k lines.
//!
//! Spec: `docs/superpowers/specs/2026-09-06-online-play-by-contact-design.md`
//! §5.3.
#![cfg(not(target_arch = "wasm32"))]

/// The one-line explanation under "Your address". Pinned by a unit test so the
/// wording cannot drift into discovery or social-network framing.
pub fn your_address_lead() -> &'static str {
    "Give this to a friend so they can invite you."
}

/// An npub abbreviated for a narrow column, keeping the `npub1` prefix (so it
/// still reads as an npub) and the last four characters (so two are
/// distinguishable at a glance). Never hex — [[feedback_npub_only_display]].
pub fn short_npub(npub: &str) -> String {
    if npub.len() > 22 {
        format!("{}…{}", &npub[..14], &npub[npub.len() - 4..])
    } else {
        npub.to_string()
    }
}

/// Draw the "Your address" block: heading, QR, the abbreviated npub, a Copy
/// button, and the lead line. `npub` is `None` for a guest, in which case the
/// block explains what signing in unlocks rather than showing nothing.
pub fn draw_your_address(ui: &mut egui::Ui, npub: Option<&str>) {
    ui.add_space(8.0);
    ui.label(
        egui::RichText::new("Your address")
            .size(16.0)
            .color(egui::Color32::from_rgb(226, 214, 178))
            .strong(),
    );
    ui.add_space(4.0);

    let Some(npub) = npub else {
        ui.label(
            egui::RichText::new(
                "Sign in with your Signet persona to get an address friends can invite.",
            )
            .size(11.0)
            .color(egui::Color32::from_rgb(150, 150, 160)),
        );
        return;
    };

    crate::menu::draw_qr(ui, npub, 132.0);
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(short_npub(npub))
                .size(11.0)
                .monospace()
                .color(egui::Color32::from_rgb(200, 200, 210)),
        );
        if ui.small_button("Copy").clicked() {
            ui.ctx().copy_text(npub.to_string());
        }
    });
    ui.add_space(2.0);
    ui.label(
        egui::RichText::new(your_address_lead())
            .size(11.0)
            .color(egui::Color32::from_rgb(150, 150, 160)),
    );
}
```

- [ ] **Step 4: Make `draw_qr` reachable and mount the panel**

In `game/engine/src/menu.rs`:

(a) change `fn draw_qr(` (line ~6276) to:

```rust
/// Render `data` as a QR. `pub(crate)` so `friends_ui` can reuse it rather than
/// growing a second QR renderer.
pub(crate) fn draw_qr(ui: &mut egui::Ui, data: &str, size_px: f32) {
```

(b) at the top of `draw_my_servers_column`'s `ScrollArea` closure — immediately
after `ui.add_space(8.0);` and **before** the `ui.label(... "My Servers" ...)`
line — insert:

```rust
            // Online play by contact §5.3 — your own address sits above the
            // servers list, because being invited is the first thing you need.
            {
                let identity = crate::signet::native_signer::load_identity();
                let npub = identity.npub();
                crate::friends_ui::draw_your_address(ui, npub.as_deref());
                ui.add_space(10.0);
                ui.separator();
            }
```

(c) register the module in `game/engine/src/main.rs`, next to `mod menu;`:

```rust
// Online play by contact — the lobby's people surface (your address, friends).
// Native only: there is no online play on the web taster.
#[cfg(not(target_arch = "wasm32"))]
mod friends_ui;
```

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo test --bin axenstax-engine friends_ui 2>&1 | tail -20
```
Expected: PASS — 4 passed.

- [ ] **Step 6: Verify clippy, then confirm the panel renders**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo clippy -- -D warnings 2>&1 | tail -20
```
Expected: no warnings. Then take a lobby screenshot with the existing flag:
```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo run -- --shot-lobby 2>&1 | tail -5
```
Expected: a screenshot is written; the My Servers column now leads with "Your
address". A guest install shows the sign-in explainer instead of a QR — that is
correct, not a failure.

- [ ] **Step 7: Commit**

```bash
git add game/engine/src/friends_ui.rs game/engine/src/menu.rs game/engine/src/main.rs && git commit -m "$(cat <<'EOF'
feat(online): "Your address" lobby panel — persona npub + QR + Copy (P1)

npub only, never hex. The lead line is pinned by a unit test so it can't drift
into discovery or social-network framing. draw_qr becomes pub(crate) so
friends_ui reuses it rather than growing a second QR renderer.

Claude-Session: https://claude.ai/code/session_011gW6q5QFBTbF1BNMHiayce
EOF
)" -- game/engine/src/friends_ui.rs game/engine/src/menu.rs game/engine/src/main.rs
```

---

## Phase 2 — Rendezvous

### Task 6: Offer/Answer payloads + NIP-44 seal/open (`rendezvous/payload.rs`)

**Files:**
- Create: `game/engine/src/rendezvous/mod.rs`
- Create: `game/engine/src/rendezvous/payload.rs`
- Modify: `game/engine/Cargo.toml` (`nostr` gains the `nip44` feature)
- Modify: `game/engine/src/main.rs`

**Interfaces:**
- Consumes: `nostr::{Event, EventBuilder, Keys, Kind, PublicKey, Tag}`, `nostr::nips::nip44::{encrypt, decrypt, Version}` — exact signatures:
  ```rust
  pub fn encrypt<T: AsRef<[u8]>>(secret_key: &SecretKey, public_key: &PublicKey, content: T, version: Version) -> Result<String, Error>;
  pub fn decrypt<T: AsRef<[u8]>>(secret_key: &SecretKey, public_key: &PublicKey, payload: T) -> Result<String, Error>;
  pub enum Version { V2 }
  ```
- Produces:
  ```rust
  pub const KIND_JOIN_OFFER: u16 = 20900;
  pub const KIND_JOIN_ANSWER: u16 = 20901;
  pub const PAYLOAD_VERSION: u32 = 1;

  #[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
  pub struct Candidate { pub kind: String, pub addr: String }

  #[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
  pub struct Offer {
      pub v: u32, pub session: String, pub persona: String,
      pub attestation: nostr::Event, pub bearer: Option<String>,
      pub protocol: u32, pub candidates: Vec<Candidate>, pub sent_at: u64,
  }
  #[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
  pub struct Answer {
      pub v: u32, pub session: String, pub persona: String,
      pub attestation: nostr::Event, pub accepted: bool, pub reason: Option<String>,
      pub protocol: u32, pub candidates: Vec<Candidate>, pub world_name: String, pub sent_at: u64,
  }
  pub fn new_session_id() -> String;                        // 32 lowercase hex chars
  pub async fn seal_offer(sender: &Keys, recipient: &PublicKey, offer: &Offer) -> Result<Event, String>;
  pub async fn seal_answer(sender: &Keys, recipient: &PublicKey, answer: &Answer) -> Result<Event, String>;
  pub fn open_offer(recipient: &Keys, ev: &Event) -> Result<Offer, String>;
  pub fn open_answer(recipient: &Keys, ev: &Event) -> Result<Answer, String>;
  pub fn recipient_of(ev: &Event) -> Option<PublicKey>;     // the `p` tag
  ```

- [ ] **Step 1: Add the `nip44` feature**

In `game/engine/Cargo.toml`, under `[target.'cfg(not(target_arch = "wasm32"))'.dependencies]`, change the `nostr` line to:

```toml
# `nip44` is already pulled in transitively (nip59 = ["nip44"]), but online play
# by contact uses NIP-44 DIRECTLY to seal the rendezvous offer/answer runtime-key
# to runtime-key — so declare it rather than inherit it. NATIVE-ONLY.
nostr = { version = "=0.44.3", features = ["nip59", "nip44"] }
```

- [ ] **Step 2: Write the failing tests**

Create `game/engine/src/rendezvous/payload.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    async fn dummy_attestation(persona: &Keys, runtime: &PublicKey) -> Event {
        crate::runtime_identity::mint_player_attestation(
            persona,
            runtime,
            nostr::Timestamp::from(1_000),
            90,
        )
        .await
        .unwrap()
    }

    async fn an_offer(persona: &Keys, runtime: &Keys) -> Offer {
        Offer {
            v: PAYLOAD_VERSION,
            session: new_session_id(),
            persona: persona.public_key().to_bech32().unwrap(),
            attestation: dummy_attestation(persona, &runtime.public_key()).await,
            bearer: Some("0f".repeat(16)),
            protocol: crate::protocol::PROTOCOL_VERSION,
            candidates: vec![
                Candidate { kind: "lan".to_string(), addr: "192.168.1.20:7700".to_string() },
                Candidate { kind: "stun".to_string(), addr: "203.0.113.9:41234".to_string() },
            ],
            sent_at: 1_700_000_000,
        }
    }

    #[test]
    fn session_ids_are_32_hex_chars_and_unique() {
        let a = new_session_id();
        assert_eq!(a.len(), 32, "16 bytes, hex");
        assert!(a.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_ne!(a, new_session_id());
    }

    #[test]
    fn offer_seals_and_opens_between_the_right_two_runtime_keys() {
        rt().block_on(async {
            let joiner_persona = Keys::generate();
            let joiner_rt = Keys::generate();
            let host_rt = Keys::generate();
            let offer = an_offer(&joiner_persona, &joiner_rt).await;

            let ev = seal_offer(&joiner_rt, &host_rt.public_key(), &offer).await.unwrap();
            assert_eq!(ev.kind, Kind::Custom(KIND_JOIN_OFFER));
            assert_eq!(ev.pubkey, joiner_rt.public_key(), "signed by the RUNTIME key");
            assert_eq!(recipient_of(&ev), Some(host_rt.public_key()), "p-tagged to the host");
            assert!(ev.verify().is_ok());

            let back = open_offer(&host_rt, &ev).unwrap();
            assert_eq!(back.session, offer.session);
            assert_eq!(back.candidates, offer.candidates);
            assert_eq!(back.bearer, offer.bearer);
            assert_eq!(back.attestation.id, offer.attestation.id);
        });
    }

    #[test]
    fn the_ciphertext_leaks_no_address_and_no_persona() {
        // The whole red-line-3 point: candidate IPs are personal data and must
        // exist ONLY inside the ciphertext, and a relay must not be able to tie
        // a runtime key to a persona.
        rt().block_on(async {
            let joiner_persona = Keys::generate();
            let joiner_rt = Keys::generate();
            let host_rt = Keys::generate();
            let offer = an_offer(&joiner_persona, &joiner_rt).await;
            let ev = seal_offer(&joiner_rt, &host_rt.public_key(), &offer).await.unwrap();

            let wire = serde_json::to_string(&ev).unwrap();
            assert!(!wire.contains("192.168.1.20"), "LAN address leaked: {wire}");
            assert!(!wire.contains("203.0.113.9"), "reflexive address leaked");
            assert!(!wire.contains(&offer.persona), "persona npub leaked");
            assert!(
                !wire.contains(&joiner_persona.public_key().to_hex()),
                "persona hex leaked"
            );
            assert!(!wire.contains(&offer.session), "session id leaked");
        });
    }

    #[test]
    fn a_third_party_cannot_open_the_offer() {
        rt().block_on(async {
            let joiner_persona = Keys::generate();
            let joiner_rt = Keys::generate();
            let host_rt = Keys::generate();
            let eavesdropper = Keys::generate();
            let offer = an_offer(&joiner_persona, &joiner_rt).await;
            let ev = seal_offer(&joiner_rt, &host_rt.public_key(), &offer).await.unwrap();
            assert!(open_offer(&eavesdropper, &ev).is_err());
        });
    }

    #[test]
    fn a_tampered_ciphertext_fails_to_open() {
        rt().block_on(async {
            let joiner_persona = Keys::generate();
            let joiner_rt = Keys::generate();
            let host_rt = Keys::generate();
            let offer = an_offer(&joiner_persona, &joiner_rt).await;
            let mut ev = seal_offer(&joiner_rt, &host_rt.public_key(), &offer).await.unwrap();
            // Flip a character in the middle of the base64 payload.
            let mut c: Vec<char> = ev.content.chars().collect();
            let mid = c.len() / 2;
            c[mid] = if c[mid] == 'A' { 'B' } else { 'A' };
            ev.content = c.into_iter().collect();
            assert!(open_offer(&host_rt, &ev).is_err(), "AEAD must reject a tampered payload");
        });
    }

    #[test]
    fn an_offer_opened_as_an_answer_is_refused_on_kind() {
        rt().block_on(async {
            let joiner_persona = Keys::generate();
            let joiner_rt = Keys::generate();
            let host_rt = Keys::generate();
            let offer = an_offer(&joiner_persona, &joiner_rt).await;
            let ev = seal_offer(&joiner_rt, &host_rt.public_key(), &offer).await.unwrap();
            assert!(open_answer(&host_rt, &ev).is_err(), "kind must gate the payload type");
        });
    }

    #[test]
    fn answer_round_trips_including_a_refusal_reason() {
        rt().block_on(async {
            let host_persona = Keys::generate();
            let host_rt = Keys::generate();
            let joiner_rt = Keys::generate();
            let answer = Answer {
                v: PAYLOAD_VERSION,
                session: new_session_id(),
                persona: host_persona.public_key().to_bech32().unwrap(),
                attestation: dummy_attestation(&host_persona, &host_rt.public_key()).await,
                accepted: false,
                reason: Some("protocol-mismatch".to_string()),
                protocol: 61,
                candidates: vec![],
                world_name: "Ivy's Hollow".to_string(),
                sent_at: 1_700_000_000,
            };
            let ev = seal_answer(&host_rt, &joiner_rt.public_key(), &answer).await.unwrap();
            assert_eq!(ev.kind, Kind::Custom(KIND_JOIN_ANSWER));
            let back = open_answer(&joiner_rt, &ev).unwrap();
            assert!(!back.accepted);
            assert_eq!(back.reason.as_deref(), Some("protocol-mismatch"));
            assert_eq!(back.world_name, "Ivy's Hollow");
        });
    }

    #[test]
    fn both_kinds_are_in_the_nip01_ephemeral_range() {
        // Ephemeral (20000..30000) means relays do not store them, which is why
        // both sides must be online — true for a join by construction.
        for k in [KIND_JOIN_OFFER, KIND_JOIN_ANSWER] {
            assert!((20_000..30_000).contains(&k), "{k} is not ephemeral");
        }
    }
}
```

- [ ] **Step 3: Run them to verify they fail**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo test --bin axenstax-engine rendezvous::payload 2>&1 | tail -20
```
Expected: FAIL — `file not found for module rendezvous`.

- [ ] **Step 4: Write the implementation**

Create `game/engine/src/rendezvous/mod.rs`:

```rust
//! The encrypted setup handshake two players exchange over public relays before
//! their machines talk directly.
//!
//! A relay sees two runtime pubkeys and a timestamp. It does not see who the
//! players are, which world is involved, or what addresses they gave each other
//! — all of that is inside NIP-44 ciphertext (CLAUDE.md red line 3). Nothing
//! here is a directory: an offer is addressed to exactly one runtime key that
//! the joiner already had, from a contact or an invite.
//!
//! Spec: `docs/superpowers/specs/2026-09-06-online-play-by-contact-design.md`
//! §3.2.
#![cfg(not(target_arch = "wasm32"))]

pub mod payload;
pub mod relay_client;
pub mod verify;
```

Create `game/engine/src/rendezvous/payload.rs` (above the test module):

```rust
//! Offer/Answer types and their NIP-44 envelope.
//!
//! The outer event is signed by the sender's **runtime** key and `p`-tagged to
//! the recipient's. The payload — including the persona attestation that ties
//! the runtime key to a person — lives entirely inside the ciphertext.

use nostr::{Event, EventBuilder, Keys, Kind, PublicKey, Tag, ToBech32};

/// Joiner → host. Ephemeral (NIP-01 20000..30000): relays do not store it.
pub const KIND_JOIN_OFFER: u16 = 20900;
/// Host → joiner. Ephemeral.
pub const KIND_JOIN_ANSWER: u16 = 20901;
/// Payload schema version. Fields are append-only.
pub const PAYLOAD_VERSION: u32 = 1;

/// One address the peer may be reachable at.
///
/// `kind` is the wire spelling of `nat::candidates::CandidateKind`
/// (`"lan" | "v6" | "upnp" | "stun"`); it is a `String` here rather than the
/// enum so an unrecognised future kind survives a round-trip instead of failing
/// the whole payload. `addr` is `ip:port`.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Candidate {
    pub kind: String,
    pub addr: String,
}

/// "I would like to join your world."
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Offer {
    pub v: u32,
    /// 16 random bytes, hex. Ties an answer to its offer and is the replay key.
    pub session: String,
    /// The joiner's persona, as an npub.
    pub persona: String,
    /// The kind-30420 `role=player` event tying `persona` to the outer signer.
    /// Carried here and never published, so no relay can correlate the two.
    pub attestation: Event,
    /// The invite bearer, hex, when joining by invite rather than as a contact.
    pub bearer: Option<String>,
    /// `protocol::PROTOCOL_VERSION`. Lets a mismatch be explained before a
    /// connect is attempted; the packets themselves are unchanged.
    pub protocol: u32,
    pub candidates: Vec<Candidate>,
    pub sent_at: u64,
}

/// "Yes, here is where to reach me" — or a refusal, for the two refusals that
/// are ever sent (`online_admission::is_reply_worthy`).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Answer {
    pub v: u32,
    pub session: String,
    pub persona: String,
    pub attestation: Event,
    pub accepted: bool,
    /// `online_admission::refusal_wire` spelling; `None` when accepted.
    pub reason: Option<String>,
    pub protocol: u32,
    pub candidates: Vec<Candidate>,
    pub world_name: String,
    pub sent_at: u64,
}

/// A fresh 16-byte session id, lowercase hex.
pub fn new_session_id() -> String {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).expect("OS RNG unavailable");
    hex::encode(b)
}

async fn seal<T: serde::Serialize>(
    sender: &Keys,
    recipient: &PublicKey,
    kind: u16,
    payload: &T,
) -> Result<Event, String> {
    let json = serde_json::to_string(payload).map_err(|e| format!("serialise payload: {e}"))?;
    let ciphertext = nostr::nips::nip44::encrypt(
        sender.secret_key(),
        recipient,
        json,
        nostr::nips::nip44::Version::V2,
    )
    .map_err(|e| format!("nip44 encrypt: {e}"))?;
    EventBuilder::new(Kind::Custom(kind), ciphertext)
        .tags([Tag::public_key(*recipient)])
        .sign(sender)
        .await
        .map_err(|e| format!("sign: {e}"))
}

fn open<T: serde::de::DeserializeOwned>(
    recipient: &Keys,
    ev: &Event,
    expect_kind: u16,
) -> Result<T, String> {
    if ev.kind != Kind::Custom(expect_kind) {
        return Err(format!("wrong kind {} (wanted {expect_kind})", ev.kind.as_u16()));
    }
    let json = nostr::nips::nip44::decrypt(recipient.secret_key(), &ev.pubkey, &ev.content)
        .map_err(|e| format!("nip44 decrypt: {e}"))?;
    serde_json::from_str(&json).map_err(|e| format!("parse payload: {e}"))
}

pub async fn seal_offer(
    sender: &Keys,
    recipient: &PublicKey,
    offer: &Offer,
) -> Result<Event, String> {
    seal(sender, recipient, KIND_JOIN_OFFER, offer).await
}

pub async fn seal_answer(
    sender: &Keys,
    recipient: &PublicKey,
    answer: &Answer,
) -> Result<Event, String> {
    seal(sender, recipient, KIND_JOIN_ANSWER, answer).await
}

pub fn open_offer(recipient: &Keys, ev: &Event) -> Result<Offer, String> {
    open(recipient, ev, KIND_JOIN_OFFER)
}

pub fn open_answer(recipient: &Keys, ev: &Event) -> Result<Answer, String> {
    open(recipient, ev, KIND_JOIN_ANSWER)
}

/// The runtime key an event is addressed to (its first `p` tag).
pub fn recipient_of(ev: &Event) -> Option<PublicKey> {
    ev.tags.iter().find_map(|t| {
        let s = t.as_slice();
        (s.len() >= 2 && s[0] == "p")
            .then(|| PublicKey::from_hex(&s[1]).ok())
            .flatten()
    })
}

/// An npub for a pubkey, falling back to hex only if bech32 encoding fails
/// (which it cannot for a valid key). Callers put this in `persona`.
pub fn npub_of(pk: &PublicKey) -> String {
    pk.to_bech32().unwrap_or_else(|_| pk.to_hex())
}
```

- [ ] **Step 5: Register the module**

In `game/engine/src/main.rs`:

```rust
// Online play by contact — the encrypted setup handshake over public relays.
// Native only (tokio websockets, NIP-44, bunker-adjacent identity).
#[cfg(not(target_arch = "wasm32"))]
mod rendezvous;
```

- [ ] **Step 6: Run the tests to verify they pass**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo test --bin axenstax-engine rendezvous::payload 2>&1 | tail -20
```
Expected: PASS — 8 passed. (The `verify` and `relay_client` submodules do not
exist yet; create them as empty files containing only `#![cfg(not(target_arch =
"wasm32"))]` and a `//!` line if the module tree refuses to compile, then fill
them in Tasks 7 and 8.)

- [ ] **Step 7: Commit**

```bash
git add game/engine/Cargo.toml game/engine/Cargo.lock game/engine/src/rendezvous game/engine/src/main.rs && git commit -m "$(cat <<'EOF'
feat(online): rendezvous Offer/Answer types + NIP-44 seal/open (P2)

Kinds 20900/20901, both ephemeral, both signed by the runtime key and p-tagged
to the peer's. Everything that identifies a person, a world, or an address is
inside the ciphertext — pinned by a test that greps the serialised event for
the addresses, the persona and the session id.

Claude-Session: https://claude.ai/code/session_011gW6q5QFBTbF1BNMHiayce
EOF
)" -- game/engine/Cargo.toml game/engine/Cargo.lock game/engine/src/rendezvous game/engine/src/main.rs
```

---

### Task 7: The verification chain (`rendezvous/verify.rs`)

**Files:**
- Create/replace: `game/engine/src/rendezvous/verify.rs`

**Interfaces:**
- Consumes: `payload::{Offer, Answer, open_offer, open_answer, PAYLOAD_VERSION}`, `runtime_identity::verify_player_attestation`, `server_identity::attestation::AttestationError`.
- Produces:
  ```rust
  pub const CLOCK_SKEW_SECS: i64 = 120;
  #[derive(Clone, Debug, PartialEq, Eq)]
  pub enum RendezvousError {
      BadOuterSignature, Envelope(String), BadVersion(u32),
      BadAttestation(String), RuntimeMismatch, PersonaMismatch,
      StaleTimestamp(i64), ReplayedSession(String),
  }
  #[derive(Default)]
  pub struct SessionGuard { /* private */ }
  impl SessionGuard {
      pub fn new() -> Self;
      pub fn check_and_insert(&mut self, session: &str, now: u64) -> bool;  // false = replay
  }
  pub struct VerifiedOffer { pub offer: Offer, pub persona: [u8; 32], pub runtime: [u8; 32] }
  pub struct VerifiedAnswer { pub answer: Answer, pub persona: [u8; 32], pub runtime: [u8; 32] }
  pub fn verify_offer(ev: &nostr::Event, me: &nostr::Keys, guard: &mut SessionGuard, now: u64)
      -> Result<VerifiedOffer, RendezvousError>;
  pub fn verify_answer(ev: &nostr::Event, me: &nostr::Keys, expect_session: &str, now: u64)
      -> Result<VerifiedAnswer, RendezvousError>;
  ```

- [ ] **Step 1: Write the failing tests**

Create `game/engine/src/rendezvous/verify.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendezvous::payload::{
        npub_of, new_session_id, seal_offer, Candidate, Offer, KIND_JOIN_OFFER, PAYLOAD_VERSION,
    };
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

    const NOW: u64 = 1_700_000_000;

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    struct Peer {
        persona: Keys,
        runtime: Keys,
    }

    impl Peer {
        fn new() -> Self {
            Peer { persona: Keys::generate(), runtime: Keys::generate() }
        }
        async fn attestation(&self) -> nostr::Event {
            crate::runtime_identity::mint_player_attestation(
                &self.persona,
                &self.runtime.public_key(),
                Timestamp::from(NOW - 10),
                90,
            )
            .await
            .unwrap()
        }
        async fn offer(&self, sent_at: u64) -> Offer {
            Offer {
                v: PAYLOAD_VERSION,
                session: new_session_id(),
                persona: npub_of(&self.persona.public_key()),
                attestation: self.attestation().await,
                bearer: None,
                protocol: crate::protocol::PROTOCOL_VERSION,
                candidates: vec![Candidate {
                    kind: "lan".to_string(),
                    addr: "10.0.0.5:7700".to_string(),
                }],
                sent_at,
            }
        }
    }

    #[test]
    fn a_well_formed_offer_verifies_and_yields_both_keys() {
        rt().block_on(async {
            let joiner = Peer::new();
            let host_rt = Keys::generate();
            let offer = joiner.offer(NOW).await;
            let ev = seal_offer(&joiner.runtime, &host_rt.public_key(), &offer)
                .await
                .unwrap();
            let mut guard = SessionGuard::new();
            let v = verify_offer(&ev, &host_rt, &mut guard, NOW).unwrap();
            assert_eq!(v.persona, joiner.persona.public_key().to_bytes());
            assert_eq!(v.runtime, joiner.runtime.public_key().to_bytes());
            assert_eq!(v.offer.session, offer.session);
        });
    }

    #[test]
    fn an_attestation_for_a_different_runtime_key_is_refused() {
        // The core forgery: take somebody else's real attestation and sign the
        // outer event with your own key.
        rt().block_on(async {
            let victim = Peer::new();
            let attacker_rt = Keys::generate();
            let host_rt = Keys::generate();
            let mut offer = victim.offer(NOW).await;
            offer.session = new_session_id();
            let ev = seal_offer(&attacker_rt, &host_rt.public_key(), &offer)
                .await
                .unwrap();
            let mut guard = SessionGuard::new();
            assert_eq!(
                verify_offer(&ev, &host_rt, &mut guard, NOW),
                Err(RendezvousError::RuntimeMismatch)
            );
        });
    }

    #[test]
    fn a_persona_field_that_disagrees_with_the_attestation_is_refused() {
        rt().block_on(async {
            let joiner = Peer::new();
            let host_rt = Keys::generate();
            let someone_else = Keys::generate();
            let mut offer = joiner.offer(NOW).await;
            offer.persona = npub_of(&someone_else.public_key());
            let ev = seal_offer(&joiner.runtime, &host_rt.public_key(), &offer)
                .await
                .unwrap();
            let mut guard = SessionGuard::new();
            assert_eq!(
                verify_offer(&ev, &host_rt, &mut guard, NOW),
                Err(RendezvousError::PersonaMismatch)
            );
        });
    }

    #[test]
    fn a_tampered_outer_event_is_refused_before_anything_else() {
        rt().block_on(async {
            let joiner = Peer::new();
            let host_rt = Keys::generate();
            let offer = joiner.offer(NOW).await;
            let mut ev = seal_offer(&joiner.runtime, &host_rt.public_key(), &offer)
                .await
                .unwrap();
            ev.created_at = Timestamp::from(NOW + 5);
            let mut guard = SessionGuard::new();
            assert_eq!(
                verify_offer(&ev, &host_rt, &mut guard, NOW),
                Err(RendezvousError::BadOuterSignature)
            );
        });
    }

    #[test]
    fn a_stale_or_future_offer_is_refused_at_the_120s_boundary() {
        rt().block_on(async {
            let joiner = Peer::new();
            let host_rt = Keys::generate();
            for (sent_at, ok) in [
                (NOW - 120, true),
                (NOW - 121, false),
                (NOW + 120, true),
                (NOW + 121, false),
            ] {
                let offer = joiner.offer(sent_at).await;
                let ev = seal_offer(&joiner.runtime, &host_rt.public_key(), &offer)
                    .await
                    .unwrap();
                let mut guard = SessionGuard::new();
                let got = verify_offer(&ev, &host_rt, &mut guard, NOW);
                assert_eq!(got.is_ok(), ok, "sent_at {sent_at} should be ok={ok}");
            }
        });
    }

    #[test]
    fn the_same_session_is_only_accepted_once() {
        rt().block_on(async {
            let joiner = Peer::new();
            let host_rt = Keys::generate();
            let offer = joiner.offer(NOW).await;
            let ev = seal_offer(&joiner.runtime, &host_rt.public_key(), &offer)
                .await
                .unwrap();
            let mut guard = SessionGuard::new();
            assert!(verify_offer(&ev, &host_rt, &mut guard, NOW).is_ok());
            assert_eq!(
                verify_offer(&ev, &host_rt, &mut guard, NOW),
                Err(RendezvousError::ReplayedSession(offer.session.clone()))
            );
        });
    }

    #[test]
    fn the_session_guard_forgets_entries_older_than_the_skew_window() {
        let mut g = SessionGuard::new();
        assert!(g.check_and_insert("aa", 1_000));
        assert!(!g.check_and_insert("aa", 1_000), "still remembered inside the window");
        // Well past 2x the skew window, the entry is pruned and the id is free
        // again — bounded memory, and a session id is not reused in practice.
        assert!(g.check_and_insert("aa", 1_000 + 2 * CLOCK_SKEW_SECS as u64 + 1));
    }

    #[test]
    fn an_unknown_payload_version_is_refused() {
        rt().block_on(async {
            let joiner = Peer::new();
            let host_rt = Keys::generate();
            let mut offer = joiner.offer(NOW).await;
            offer.v = 99;
            let ev = seal_offer(&joiner.runtime, &host_rt.public_key(), &offer)
                .await
                .unwrap();
            let mut guard = SessionGuard::new();
            assert_eq!(
                verify_offer(&ev, &host_rt, &mut guard, NOW),
                Err(RendezvousError::BadVersion(99))
            );
        });
    }

    #[test]
    fn an_offer_with_a_server_attestation_is_refused() {
        rt().block_on(async {
            let joiner = Peer::new();
            let host_rt = Keys::generate();
            let mut offer = joiner.offer(NOW).await;
            // A role-less (server) attestation over the same runtime key.
            offer.attestation = crate::server_identity::attestation::mint_attestation(
                &joiner.persona,
                &joiner.runtime.public_key(),
                Timestamp::from(NOW - 10),
                Timestamp::from(NOW + 10_000),
                &[27420],
                "",
                None,
            )
            .await
            .unwrap();
            let ev = seal_offer(&joiner.runtime, &host_rt.public_key(), &offer)
                .await
                .unwrap();
            let mut guard = SessionGuard::new();
            assert!(matches!(
                verify_offer(&ev, &host_rt, &mut guard, NOW),
                Err(RendezvousError::BadAttestation(_))
            ));
        });
    }

    #[test]
    fn an_answer_for_a_different_session_is_refused() {
        rt().block_on(async {
            use crate::rendezvous::payload::{seal_answer, Answer};
            let host = Peer::new();
            let joiner_rt = Keys::generate();
            let answer = Answer {
                v: PAYLOAD_VERSION,
                session: new_session_id(),
                persona: npub_of(&host.persona.public_key()),
                attestation: host.attestation().await,
                accepted: true,
                reason: None,
                protocol: crate::protocol::PROTOCOL_VERSION,
                candidates: vec![],
                world_name: "W".to_string(),
                sent_at: NOW,
            };
            let ev = seal_answer(&host.runtime, &joiner_rt.public_key(), &answer)
                .await
                .unwrap();
            assert!(verify_answer(&ev, &joiner_rt, &answer.session, NOW).is_ok());
            assert_eq!(
                verify_answer(&ev, &joiner_rt, "deadbeef", NOW),
                Err(RendezvousError::ReplayedSession(answer.session.clone())),
            );
        });
    }

    #[test]
    fn an_event_of_the_wrong_kind_is_refused_by_the_envelope() {
        rt().block_on(async {
            let joiner = Peer::new();
            let host_rt = Keys::generate();
            let ev = EventBuilder::new(Kind::Custom(1), "hello")
                .tags([Tag::public_key(host_rt.public_key())])
                .sign(&joiner.runtime)
                .await
                .unwrap();
            let mut guard = SessionGuard::new();
            assert!(matches!(
                verify_offer(&ev, &host_rt, &mut guard, NOW),
                Err(RendezvousError::Envelope(_))
            ));
            assert_ne!(ev.kind, Kind::Custom(KIND_JOIN_OFFER));
        });
    }
}
```

- [ ] **Step 2: Run them to verify they fail**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo test --bin axenstax-engine rendezvous::verify 2>&1 | tail -20
```
Expected: FAIL — `cannot find type SessionGuard` etc.

- [ ] **Step 3: Write the implementation**

Put this above the test module in `game/engine/src/rendezvous/verify.rs`:

```rust
//! The verification chain every inbound offer and answer runs through, and the
//! replay guard.
//!
//! The chain, in order, and why each link is there:
//!
//! 1. **Outer signature** — the event really was signed by the key that claims
//!    to have signed it. Cheapest check that can reject a forgery, so it is
//!    first.
//! 2. **Kind + decrypt + parse + version** — the envelope is one of ours and
//!    the payload is a shape we understand.
//! 3. **Attestation** — the enclosed kind-30420 `role=player` event is validly
//!    signed by the persona it names, is inside its validity window, and names
//!    **the outer signer** as its runtime key. Without link 3's last clause,
//!    anybody could replay somebody else's attestation under their own key.
//! 4. **Persona agreement** — the payload's `persona` npub is the attestation's
//!    signer. Without this the payload could name one person while proving
//!    another.
//! 5. **Freshness** — `sent_at` within ±120 s of now, so a captured offer is
//!    not useful tomorrow.
//! 6. **Session unseen** — the same session id is admitted once.
//!
//! Any failure is a **silent drop** at the call site: the host never answers a
//! stranger (spec §3.3). The distinct error variants exist for logs and tests,
//! not for a reply.

use std::collections::HashMap;

use nostr::{Event, Keys, PublicKey};

use crate::rendezvous::payload::{
    open_answer, open_offer, Answer, Offer, PAYLOAD_VERSION,
};
use crate::runtime_identity::verify_player_attestation;

/// How far an offer/answer's `sent_at` may sit from the receiver's clock.
/// Generous enough for a household router's idea of the time, tight enough that
/// a captured payload is stale within minutes.
pub const CLOCK_SKEW_SECS: i64 = 120;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RendezvousError {
    /// The outer event's signature does not match its id/pubkey.
    BadOuterSignature,
    /// Wrong kind, or the ciphertext did not open, or the JSON did not parse.
    Envelope(String),
    BadVersion(u32),
    /// The enclosed attestation did not verify (bad signature, expired, not a
    /// `role=player` attestation, …).
    BadAttestation(String),
    /// The attestation names a runtime key that is not the outer signer.
    RuntimeMismatch,
    /// The payload's `persona` is not the attestation's signer.
    PersonaMismatch,
    /// `sent_at` outside ±[`CLOCK_SKEW_SECS`]; carries the signed delta.
    StaleTimestamp(i64),
    /// This session id has already been handled (or, for an answer, is not the
    /// session we are waiting on).
    ReplayedSession(String),
}

/// Remembers session ids briefly so the same offer cannot be processed twice.
///
/// Bounded by pruning on every insert: an id older than twice the skew window
/// can no longer be part of a fresh payload, so forgetting it is safe.
#[derive(Default)]
pub struct SessionGuard {
    seen: HashMap<String, u64>,
}

impl SessionGuard {
    pub fn new() -> Self {
        Self::default()
    }

    /// `true` if this session is new (and is now remembered); `false` if it was
    /// already seen inside the window.
    pub fn check_and_insert(&mut self, session: &str, now: u64) -> bool {
        let horizon = 2 * CLOCK_SKEW_SECS as u64;
        self.seen.retain(|_, t| now.saturating_sub(*t) <= horizon);
        if self.seen.contains_key(session) {
            return false;
        }
        self.seen.insert(session.to_string(), now);
        true
    }
}

pub struct VerifiedOffer {
    pub offer: Offer,
    /// The joiner's persona, x-only bytes.
    pub persona: [u8; 32],
    /// The joiner's runtime key (= the outer signer), x-only bytes.
    pub runtime: [u8; 32],
}

pub struct VerifiedAnswer {
    pub answer: Answer,
    pub persona: [u8; 32],
    pub runtime: [u8; 32],
}

/// Links 1, 3, 4, 5 — everything that does not depend on which payload type
/// this is. Returns the attesting persona.
fn verify_common(
    ev: &Event,
    attestation: &Event,
    claimed_persona: &str,
    v: u32,
    sent_at: u64,
    now: u64,
) -> Result<PublicKey, RendezvousError> {
    if v != PAYLOAD_VERSION {
        return Err(RendezvousError::BadVersion(v));
    }
    let persona = verify_player_attestation(
        attestation,
        &ev.pubkey,
        nostr::Timestamp::from(now),
    )
    .map_err(|e| match e {
        crate::server_identity::attestation::AttestationError::WrongServerKey => {
            RendezvousError::RuntimeMismatch
        }
        other => RendezvousError::BadAttestation(format!("{other:?}")),
    })?;

    let claimed = PublicKey::parse(claimed_persona)
        .map_err(|_| RendezvousError::PersonaMismatch)?;
    if claimed != persona {
        return Err(RendezvousError::PersonaMismatch);
    }

    let delta = sent_at as i64 - now as i64;
    if delta.abs() > CLOCK_SKEW_SECS {
        return Err(RendezvousError::StaleTimestamp(delta));
    }
    Ok(persona)
}

/// Verify an inbound kind-20900 addressed to `me`. On any `Err` the caller
/// drops the event without answering.
pub fn verify_offer(
    ev: &Event,
    me: &Keys,
    guard: &mut SessionGuard,
    now: u64,
) -> Result<VerifiedOffer, RendezvousError> {
    if ev.verify().is_err() {
        return Err(RendezvousError::BadOuterSignature);
    }
    let offer = open_offer(me, ev).map_err(RendezvousError::Envelope)?;
    let persona = verify_common(
        ev,
        &offer.attestation,
        &offer.persona,
        offer.v,
        offer.sent_at,
        now,
    )?;
    if !guard.check_and_insert(&offer.session, now) {
        return Err(RendezvousError::ReplayedSession(offer.session));
    }
    Ok(VerifiedOffer {
        persona: persona.to_bytes(),
        runtime: ev.pubkey.to_bytes(),
        offer,
    })
}

/// Verify an inbound kind-20901 addressed to `me`, for the session we are
/// actually waiting on. An answer for any other session is treated as a replay
/// — a joiner has exactly one outstanding call.
pub fn verify_answer(
    ev: &Event,
    me: &Keys,
    expect_session: &str,
    now: u64,
) -> Result<VerifiedAnswer, RendezvousError> {
    if ev.verify().is_err() {
        return Err(RendezvousError::BadOuterSignature);
    }
    let answer = open_answer(me, ev).map_err(RendezvousError::Envelope)?;
    let persona = verify_common(
        ev,
        &answer.attestation,
        &answer.persona,
        answer.v,
        answer.sent_at,
        now,
    )?;
    if answer.session != expect_session {
        return Err(RendezvousError::ReplayedSession(answer.session));
    }
    Ok(VerifiedAnswer {
        persona: persona.to_bytes(),
        runtime: ev.pubkey.to_bytes(),
        answer,
    })
}
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo test --bin axenstax-engine rendezvous:: 2>&1 | tail -20
```
Expected: PASS — 11 verify tests + the 8 payload tests.

- [ ] **Step 5: Commit**

```bash
git add game/engine/src/rendezvous/verify.rs && git commit -m "$(cat <<'EOF'
feat(online): rendezvous verification chain + replay guard (P2)

Six links, in cheapest-first order: outer signature, envelope, payload version,
attestation (must name the OUTER SIGNER as its runtime key), persona agreement,
±120s freshness, session-seen-once. Every failure is a silent drop at the call
site — the host never answers a stranger.

Claude-Session: https://claude.ai/code/session_011gW6q5QFBTbF1BNMHiayce
EOF
)" -- game/engine/src/rendezvous/verify.rs
```

---

### Task 8: Relay client — trait, in-memory fake, live worker (`rendezvous/relay_client.rs`)

**Files:**
- Create/replace: `game/engine/src/rendezvous/relay_client.rs`

**Interfaces:**
- Consumes:
  ```rust
  // src/native_mailbox/relay.rs — reuse the PARSER, not the REQ builder
  pub enum Frame { Wrap(Box<nostr::Event>), Eose, Ok { id: String, accepted: bool }, Other }
  pub fn parse_frame(text: &str, sub_id: &str) -> Frame;
  pub fn event_frame(ev: &nostr::Event) -> String;
  ```
  (`native_mailbox::relay::req_frame` hardcodes `kinds:[1059]`, so this task
  writes its own `req_frame` and reuses `parse_frame`/`event_frame`.)
- Produces:
  ```rust
  pub trait RendezvousRelay: Send {
      fn publish(&self, ev: &nostr::Event) -> Result<(), String>;
      fn subscribe(&self, kind: u16, p_tag_hex: &str) -> Result<(), String>;
      fn try_recv(&self) -> Option<nostr::Event>;
      fn connected(&self) -> (usize, usize);   // (connected, total)
  }
  pub fn req_frame(sub_id: &str, kind: u16, p_tag_hex: &str, since: u64) -> String;

  pub struct FakeRelayHub { /* private */ }
  impl FakeRelayHub {
      pub fn new() -> Self;
      pub fn client(&self) -> FakeRelay;
      pub fn delivered(&self) -> usize;
  }
  pub struct FakeRelay { /* private */ }
  impl RendezvousRelay for FakeRelay { /* … */ }

  pub struct MultiRelay { /* private */ }
  impl MultiRelay { pub fn start(urls: Vec<String>) -> MultiRelay; }
  impl RendezvousRelay for MultiRelay { /* … */ }
  ```

- [ ] **Step 1: Write the failing tests**

Create `game/engine/src/rendezvous/relay_client.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    async fn addressed_to(sender: &Keys, recipient: &nostr::PublicKey, kind: u16) -> nostr::Event {
        EventBuilder::new(Kind::Custom(kind), "ciphertext")
            .tags([Tag::public_key(*recipient)])
            .sign(sender)
            .await
            .unwrap()
    }

    #[test]
    fn req_frame_matches_the_known_good_relay_shape() {
        // Same field order as tools/feedback-reader/live.mjs and
        // native_mailbox::relay::req_frame — trotters rejects nostr-tools'
        // framing of the same filter, so the order is load-bearing.
        let f = req_frame("rz", 20900, &"ab".repeat(32), 500);
        let v: serde_json::Value = serde_json::from_str(&f).unwrap();
        assert_eq!(v[0], "REQ");
        assert_eq!(v[1], "rz");
        assert_eq!(v[2]["kinds"][0], 20900);
        assert_eq!(v[2]["#p"][0], "ab".repeat(32));
        assert_eq!(v[2]["since"], 500);
    }

    #[test]
    fn the_fake_delivers_only_to_a_matching_subscriber() {
        rt().block_on(async {
            let hub = FakeRelayHub::new();
            let host = hub.client();
            let joiner = hub.client();
            let host_key = Keys::generate();
            let joiner_key = Keys::generate();

            host.subscribe(20900, &host_key.public_key().to_hex()).unwrap();
            joiner.subscribe(20901, &joiner_key.public_key().to_hex()).unwrap();

            // Offer → host.
            let offer = addressed_to(&joiner_key, &host_key.public_key(), 20900).await;
            joiner.publish(&offer).unwrap();
            assert_eq!(host.try_recv().map(|e| e.id), Some(offer.id));
            assert!(joiner.try_recv().is_none(), "not addressed to the joiner");

            // Answer → joiner.
            let answer = addressed_to(&host_key, &joiner_key.public_key(), 20901).await;
            host.publish(&answer).unwrap();
            assert_eq!(joiner.try_recv().map(|e| e.id), Some(answer.id));
            assert!(host.try_recv().is_none());
        });
    }

    #[test]
    fn the_fake_drops_an_event_of_the_wrong_kind() {
        rt().block_on(async {
            let hub = FakeRelayHub::new();
            let host = hub.client();
            let other = hub.client();
            let host_key = Keys::generate();
            host.subscribe(20900, &host_key.public_key().to_hex()).unwrap();
            // Right recipient, wrong kind.
            let ev = addressed_to(&Keys::generate(), &host_key.public_key(), 20901).await;
            other.publish(&ev).unwrap();
            assert!(host.try_recv().is_none());
        });
    }

    #[test]
    fn the_fake_drops_an_event_addressed_to_somebody_else() {
        rt().block_on(async {
            let hub = FakeRelayHub::new();
            let host = hub.client();
            let other = hub.client();
            let host_key = Keys::generate();
            host.subscribe(20900, &host_key.public_key().to_hex()).unwrap();
            let ev = addressed_to(&Keys::generate(), &Keys::generate().public_key(), 20900).await;
            other.publish(&ev).unwrap();
            assert!(host.try_recv().is_none());
        });
    }

    #[test]
    fn the_fake_delivers_in_order_and_reports_a_delivery_count() {
        rt().block_on(async {
            let hub = FakeRelayHub::new();
            let host = hub.client();
            let joiner = hub.client();
            let host_key = Keys::generate();
            host.subscribe(20900, &host_key.public_key().to_hex()).unwrap();
            let a = addressed_to(&Keys::generate(), &host_key.public_key(), 20900).await;
            let b = addressed_to(&Keys::generate(), &host_key.public_key(), 20900).await;
            joiner.publish(&a).unwrap();
            joiner.publish(&b).unwrap();
            assert_eq!(hub.delivered(), 2);
            assert_eq!(host.try_recv().map(|e| e.id), Some(a.id));
            assert_eq!(host.try_recv().map(|e| e.id), Some(b.id));
            assert!(host.try_recv().is_none());
        });
    }

    #[test]
    fn the_fake_reports_itself_as_one_of_one_connected() {
        let hub = FakeRelayHub::new();
        assert_eq!(hub.client().connected(), (1, 1));
    }

    /// OWNER BOUNDARY (live relay) — needs network, not run in CI.
    /// Run manually: `cargo test --bin axenstax-engine relay_client -- --ignored`
    #[test]
    #[ignore]
    fn live_multi_relay_connects_and_subscribes() {
        let m = MultiRelay::start(vec!["wss://relay.trotters.cc".to_string()]);
        m.subscribe(20900, &"ab".repeat(32)).unwrap();
        // Give the worker a moment to complete the TLS + REQ round-trip.
        std::thread::sleep(std::time::Duration::from_secs(4));
        assert_eq!(m.connected(), (1, 1), "the worker should have one live relay");
    }
}
```

- [ ] **Step 2: Run them to verify they fail**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo test --bin axenstax-engine rendezvous::relay_client 2>&1 | tail -20
```
Expected: FAIL — `cannot find type FakeRelayHub`.

- [ ] **Step 3: Write the implementation**

Put this above the test module in `game/engine/src/rendezvous/relay_client.rs`:

```rust
//! Talking to relays — behind a trait, so every state machine above it is
//! testable with no network at all.
//!
//! Two implementations: [`MultiRelay`] (a worker thread holding one websocket
//! per relay, reconnecting with backoff) and [`FakeRelayHub`]/[`FakeRelay`] (an
//! in-memory bus two clients share). The fake is not a stub: it applies the same
//! `kind` + `#p` filter a real relay applies, so a test that passes against it
//! is testing the filter as well as the flow.
//!
//! Frames use the raw REQ/EVENT array shape that `tools/feedback-reader/live.mjs`
//! and `native_mailbox::relay` already use successfully against
//! `wss://relay.trotters.cc` — nostr-tools' framing of the same filter is
//! rejected there, so the field order is load-bearing.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use nostr::Event;
use serde_json::json;
use tokio_tungstenite::tungstenite::Message;

use crate::native_mailbox::relay::{event_frame, parse_frame, Frame};

/// What the rendezvous needs from a relay set. Deliberately tiny: publish one
/// event, subscribe to one (kind, recipient) filter, drain what arrived.
pub trait RendezvousRelay: Send {
    fn publish(&self, ev: &Event) -> Result<(), String>;
    fn subscribe(&self, kind: u16, p_tag_hex: &str) -> Result<(), String>;
    /// Non-blocking: the next event that matched a subscription, if any.
    fn try_recv(&self) -> Option<Event>;
    /// `(connected, total)` — shown on the Online panel so a host can see at a
    /// glance whether they are actually reachable.
    fn connected(&self) -> (usize, usize);
}

/// The REQ frame subscribing to `kind` events addressed to `p_tag_hex`.
///
/// `since` rather than `limit`: these kinds are ephemeral so there is nothing
/// stored to page through, and `limit: 0` is refused by some relays.
pub fn req_frame(sub_id: &str, kind: u16, p_tag_hex: &str, since: u64) -> String {
    json!(["REQ", sub_id, {
        "kinds": [kind],
        "#p": [p_tag_hex],
        "since": since,
    }])
    .to_string()
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ─── The in-memory fake ─────────────────────────────────────────────────────

struct HubInner {
    /// `(client_index, kind, p_tag_hex)`.
    subs: Vec<(usize, u16, String)>,
    inboxes: Vec<VecDeque<Event>>,
    delivered: usize,
}

/// An in-memory stand-in for a relay set, shared by every [`FakeRelay`] it
/// hands out. Filtering is real, so the tests above prove the filter too.
pub struct FakeRelayHub {
    inner: Arc<Mutex<HubInner>>,
}

impl Default for FakeRelayHub {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeRelayHub {
    pub fn new() -> Self {
        FakeRelayHub {
            inner: Arc::new(Mutex::new(HubInner {
                subs: Vec::new(),
                inboxes: Vec::new(),
                delivered: 0,
            })),
        }
    }

    /// A new client attached to this hub.
    pub fn client(&self) -> FakeRelay {
        let mut g = self.inner.lock().expect("hub mutex");
        g.inboxes.push(VecDeque::new());
        let id = g.inboxes.len() - 1;
        FakeRelay {
            inner: Arc::clone(&self.inner),
            id,
        }
    }

    /// How many events the hub has routed to at least one subscriber. Lets a
    /// test assert "the offer actually went somewhere" without polling.
    pub fn delivered(&self) -> usize {
        self.inner.lock().expect("hub mutex").delivered
    }
}

pub struct FakeRelay {
    inner: Arc<Mutex<HubInner>>,
    id: usize,
}

impl RendezvousRelay for FakeRelay {
    fn publish(&self, ev: &Event) -> Result<(), String> {
        let recipient = crate::rendezvous::payload::recipient_of(ev).map(|p| p.to_hex());
        let mut g = self.inner.lock().map_err(|_| "hub mutex".to_string())?;
        let targets: Vec<usize> = g
            .subs
            .iter()
            .filter(|(_, kind, p)| {
                ev.kind == nostr::Kind::Custom(*kind) && recipient.as_deref() == Some(p.as_str())
            })
            .map(|(client, _, _)| *client)
            .collect();
        if !targets.is_empty() {
            g.delivered += 1;
        }
        for t in targets {
            if let Some(inbox) = g.inboxes.get_mut(t) {
                inbox.push_back(ev.clone());
            }
        }
        Ok(())
    }

    fn subscribe(&self, kind: u16, p_tag_hex: &str) -> Result<(), String> {
        let mut g = self.inner.lock().map_err(|_| "hub mutex".to_string())?;
        g.subs.push((self.id, kind, p_tag_hex.to_string()));
        Ok(())
    }

    fn try_recv(&self) -> Option<Event> {
        let mut g = self.inner.lock().ok()?;
        g.inboxes.get_mut(self.id)?.pop_front()
    }

    fn connected(&self) -> (usize, usize) {
        (1, 1)
    }
}

// ─── The live worker ────────────────────────────────────────────────────────

enum Cmd {
    Publish(Box<Event>),
    Subscribe { kind: u16, p: String },
}

/// One worker thread holding a websocket per relay. Commands go in on a
/// channel, matching events come back on another.
///
/// OWNER BOUNDARY: the socket half is verified against a live relay, not in CI.
/// The frame shapes it sends are unit-tested above and in `native_mailbox`.
pub struct MultiRelay {
    cmd_tx: mpsc::Sender<Cmd>,
    ev_rx: Mutex<mpsc::Receiver<Event>>,
    connected: Arc<AtomicUsize>,
    total: usize,
    shutdown: Arc<AtomicBool>,
}

impl MultiRelay {
    /// Start the worker. Returns immediately; connections come up in the
    /// background and `connected()` reports progress.
    pub fn start(urls: Vec<String>) -> MultiRelay {
        let (cmd_tx, cmd_rx) = mpsc::channel::<Cmd>();
        let (ev_tx, ev_rx) = mpsc::channel::<Event>();
        let connected = Arc::new(AtomicUsize::new(0));
        let shutdown = Arc::new(AtomicBool::new(false));
        let total = urls.len();

        let worker_connected = Arc::clone(&connected);
        let worker_shutdown = Arc::clone(&shutdown);
        std::thread::Builder::new()
            .name("rendezvous-relay".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        log::error!("[rendezvous] no tokio runtime: {e}");
                        return;
                    }
                };
                rt.block_on(relay_worker(
                    urls,
                    cmd_rx,
                    ev_tx,
                    worker_connected,
                    worker_shutdown,
                ));
            })
            .expect("spawn rendezvous relay thread");

        MultiRelay {
            cmd_tx,
            ev_rx: Mutex::new(ev_rx),
            connected,
            total,
            shutdown,
        }
    }
}

impl Drop for MultiRelay {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }
}

impl RendezvousRelay for MultiRelay {
    fn publish(&self, ev: &Event) -> Result<(), String> {
        self.cmd_tx
            .send(Cmd::Publish(Box::new(ev.clone())))
            .map_err(|_| "relay worker is gone".to_string())
    }

    fn subscribe(&self, kind: u16, p_tag_hex: &str) -> Result<(), String> {
        self.cmd_tx
            .send(Cmd::Subscribe {
                kind,
                p: p_tag_hex.to_string(),
            })
            .map_err(|_| "relay worker is gone".to_string())
    }

    fn try_recv(&self) -> Option<Event> {
        self.ev_rx.lock().ok()?.try_recv().ok()
    }

    fn connected(&self) -> (usize, usize) {
        (self.connected.load(Ordering::Relaxed), self.total)
    }
}

/// Connect to every relay, replay the standing subscription to each as it comes
/// up, forward matching events, and reconnect with capped backoff. Commands are
/// polled rather than awaited so one slow relay cannot stall the others.
async fn relay_worker(
    urls: Vec<String>,
    cmd_rx: mpsc::Receiver<Cmd>,
    ev_tx: mpsc::Sender<Event>,
    connected: Arc<AtomicUsize>,
    shutdown: Arc<AtomicBool>,
) {
    const SUB_ID: &str = "rz";
    let mut sockets: Vec<Option<_>> = (0..urls.len()).map(|_| None).collect();
    let mut next_try = vec![std::time::Instant::now(); urls.len()];
    let mut backoff = vec![Duration::from_secs(1); urls.len()];
    let mut standing: Option<(u16, String)> = None;

    while !shutdown.load(Ordering::Relaxed) {
        // 1. (Re)connect anything that is down and due.
        for i in 0..urls.len() {
            if sockets[i].is_some() || std::time::Instant::now() < next_try[i] {
                continue;
            }
            match tokio_tungstenite::connect_async(&urls[i]).await {
                Ok((ws, _)) => {
                    log::info!("[rendezvous] connected {}", urls[i]);
                    sockets[i] = Some(ws);
                    backoff[i] = Duration::from_secs(1);
                    if let (Some((kind, p)), Some(ws)) = (standing.clone(), sockets[i].as_mut()) {
                        let _ = ws
                            .send(Message::Text(req_frame(SUB_ID, kind, &p, unix_now())))
                            .await;
                    }
                }
                Err(e) => {
                    log::warn!("[rendezvous] {} unreachable: {e}", urls[i]);
                    next_try[i] = std::time::Instant::now() + backoff[i];
                    backoff[i] = (backoff[i] * 2).min(Duration::from_secs(60));
                }
            }
        }
        connected.store(
            sockets.iter().filter(|s| s.is_some()).count(),
            Ordering::Relaxed,
        );

        // 2. Apply any pending commands to every live socket.
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                Cmd::Subscribe { kind, p } => {
                    standing = Some((kind, p.clone()));
                    let frame = req_frame(SUB_ID, kind, &p, unix_now());
                    for ws in sockets.iter_mut().flatten() {
                        let _ = ws.send(Message::Text(frame.clone())).await;
                    }
                }
                Cmd::Publish(ev) => {
                    let frame = event_frame(&ev);
                    for ws in sockets.iter_mut().flatten() {
                        let _ = ws.send(Message::Text(frame.clone())).await;
                    }
                }
            }
        }

        // 3. Drain whatever arrived, with a short timeout so the loop keeps
        //    turning even when every relay is quiet.
        for i in 0..sockets.len() {
            let Some(ws) = sockets[i].as_mut() else { continue };
            match tokio::time::timeout(Duration::from_millis(30), ws.next()).await {
                Ok(Some(Ok(Message::Text(t)))) => {
                    if let Frame::Wrap(ev) = parse_frame(&t, SUB_ID) {
                        let _ = ev_tx.send(*ev);
                    }
                }
                // Closed or errored: drop it so step 1 reconnects it.
                Ok(Some(Err(_)) | None) => {
                    log::info!("[rendezvous] {} closed", urls[i]);
                    sockets[i] = None;
                    next_try[i] = std::time::Instant::now() + backoff[i];
                }
                // Ping/pong/binary, or simply nothing this turn.
                Ok(Some(Ok(_))) | Err(_) => {}
            }
        }

        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    for ws in sockets.iter_mut().flatten() {
        let _ = ws.close(None).await;
    }
    log::info!("[rendezvous] relay worker stopped");
}
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo test --bin axenstax-engine rendezvous::relay_client 2>&1 | tail -20
```
Expected: PASS — 6 passed, 1 ignored (the live-relay one).

- [ ] **Step 5: Verify clippy**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo clippy -- -D warnings 2>&1 | tail -20
```
Expected: no warnings. (`native_mailbox::relay::{parse_frame, event_frame}` are
`#[allow(dead_code)]` today; importing them here is fine and the attribute can
stay.)

- [ ] **Step 6: Commit**

```bash
git add game/engine/src/rendezvous/relay_client.rs && git commit -m "$(cat <<'EOF'
feat(online): relay client trait, in-memory fake, multi-relay worker (P2)

The fake applies the same kind + #p filter a real relay does, so every state
machine above this trait is testable with no network while still exercising the
filter. The live worker holds one socket per relay, replays the standing
subscription on reconnect, and backs off to 60s.

Claude-Session: https://claude.ai/code/session_011gW6q5QFBTbF1BNMHiayce
EOF
)" -- game/engine/src/rendezvous/relay_client.rs
```

---

### Task 9: Online settings — relays and port (`graphics_settings.rs`)

**Files:**
- Modify: `game/engine/src/graphics_settings.rs`
- Modify: `game/engine/src/menu.rs` (the Settings panel's new **Online** heading)

**Interfaces:**
- Consumes: the existing persistence pattern — `GraphicsSettings::load()` /
  `save()` over `load_raw`/`save_raw` (native: `settings.json` in the working
  directory; wasm: `localStorage` key `axenstax_gfx`), plus `migrate()` and
  `clamp()` which run on every load.
- Produces:
  ```rust
  pub struct GraphicsSettings { /* … existing fields … */
      pub online_relays: Vec<String>,
      pub online_port: u16,
  }
  pub fn default_online_relays() -> Vec<String>;
  pub const DEFAULT_ONLINE_PORT: u16 = 7700;
  pub fn sanitise_relays(relays: Vec<String>) -> Vec<String>;
  ```

- [ ] **Step 1: Write the failing tests**

Append to the `mod tests` block at the bottom of `game/engine/src/graphics_settings.rs`:

```rust
    // ─── Online play by contact (spec §6) ───

    #[test]
    fn default_relays_are_the_four_from_the_spec_in_order() {
        assert_eq!(
            default_online_relays(),
            vec![
                // TEST-ONLY — remove before public launch (spec §1 red line 2).
                "wss://relay.trotters.cc".to_string(),
                "wss://nos.lol".to_string(),
                "wss://relay.damus.io".to_string(),
                "wss://relay.primal.net".to_string(),
            ]
        );
    }

    #[test]
    fn a_fresh_settings_object_carries_the_online_defaults() {
        let s = GraphicsSettings::default();
        assert_eq!(s.online_relays, default_online_relays());
        assert_eq!(s.online_port, DEFAULT_ONLINE_PORT);
    }

    #[test]
    fn sanitise_drops_non_wss_blank_and_duplicate_relays_and_caps_at_eight() {
        let got = sanitise_relays(vec![
            "  wss://a.example  ".to_string(),
            "ws://insecure.example".to_string(),
            "".to_string(),
            "https://not-a-relay.example".to_string(),
            "wss://a.example".to_string(),
            "wss://b.example".to_string(),
        ]);
        assert_eq!(
            got,
            vec!["wss://a.example".to_string(), "wss://b.example".to_string()]
        );

        let many: Vec<String> = (0..12).map(|i| format!("wss://r{i}.example")).collect();
        assert_eq!(sanitise_relays(many).len(), 8, "capped at the invite's MAX_RELAYS");
    }

    #[test]
    fn clamp_repairs_an_empty_relay_list_but_keeps_a_custom_one() {
        let mut s = GraphicsSettings { online_relays: vec![], ..Default::default() };
        s.clamp();
        assert_eq!(
            s.online_relays,
            default_online_relays(),
            "an empty list would make online play silently impossible"
        );

        let mut custom = GraphicsSettings {
            online_relays: vec!["wss://mine.example".to_string()],
            ..Default::default()
        };
        custom.clamp();
        assert_eq!(custom.online_relays, vec!["wss://mine.example".to_string()]);
    }

    #[test]
    fn an_old_settings_file_without_the_online_fields_still_loads() {
        // `serde(default)` is what keeps an existing settings.json readable —
        // pinned here because losing it would reset every player's graphics
        // preferences, not just their relays. The "old" file is built by
        // serialising today's settings and DELETING the two new keys, so this
        // test cannot rot as other fields are added.
        let mut v: serde_json::Value =
            serde_json::to_value(GraphicsSettings::default()).unwrap();
        let obj = v.as_object_mut().unwrap();
        obj.remove("online_relays");
        obj.remove("online_port");
        assert!(!obj.contains_key("online_relays"));

        let s: GraphicsSettings = serde_json::from_value(v).unwrap();
        assert_eq!(
            s.render_distance,
            GraphicsSettings::default().render_distance,
            "the old fields survive"
        );
        assert_eq!(s.online_relays, default_online_relays());
        assert_eq!(s.online_port, DEFAULT_ONLINE_PORT);
    }

    #[test]
    fn online_settings_are_not_part_of_preset_detection() {
        // Relays and a port are not GPU load. Editing them must not knock the
        // player off "High" — the same rule FOV and sensitivity follow.
        let mut s = GraphicsSettings::from_preset(GraphicsPreset::High);
        s.online_relays = vec!["wss://mine.example".to_string()];
        s.online_port = 0;
        assert_eq!(s.preset(), GraphicsPreset::High);
    }
```

- [ ] **Step 2: Run them to verify they fail**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo test --bin axenstax-engine graphics_settings 2>&1 | tail -20
```
Expected: FAIL — `cannot find function default_online_relays`.

- [ ] **Step 3: Write the implementation**

In `game/engine/src/graphics_settings.rs`:

(a) add the two fields at the end of `pub struct GraphicsSettings` (after
`has_seen_license_onboarding`):

```rust
    /// Online play by contact (spec §6) — the relays the rendezvous handshake
    /// is published on. **Setup only**: a relay sees two runtime pubkeys and a
    /// timestamp and never carries a byte of game traffic (CLAUDE.md red line
    /// 2). Editable so a household can run its own. `serde(default)` so an
    /// existing settings.json still loads.
    #[serde(default = "default_online_relays")]
    pub online_relays: Vec<String>,
    /// UDP port to bind when hosting online. `0` = let the OS choose an
    /// ephemeral port, which is fine (the port travels inside the candidates)
    /// but makes a manual router forward impossible, so the default is the
    /// familiar `protocol::SERVER_PORT`.
    #[serde(default = "default_online_port")]
    pub online_port: u16,
```

(b) add the defaults, near `STORAGE_KEY`:

```rust
/// The shipped relay set for the rendezvous handshake.
///
/// `relay.trotters.cc` is ours and is here **for testing only — remove it
/// before public launch** (spec §1 red line 2: we operate no relay that a
/// group's traffic depends on, and being in the default list makes it look
/// like infrastructure it must not become).
pub fn default_online_relays() -> Vec<String> {
    vec![
        "wss://relay.trotters.cc".to_string(),
        "wss://nos.lol".to_string(),
        "wss://relay.damus.io".to_string(),
        "wss://relay.primal.net".to_string(),
    ]
}

/// Same as `protocol::SERVER_PORT` — the port a LAN host already uses, so a
/// household that has forwarded it once has forwarded it for both.
pub const DEFAULT_ONLINE_PORT: u16 = 7700;

fn default_online_port() -> u16 {
    DEFAULT_ONLINE_PORT
}

/// Trim, drop anything that isn't `wss://`, de-duplicate, and cap at 8 (the
/// same ceiling `invite::MAX_RELAYS` enforces on a pasted link, so a host can
/// never mint an invite its own settings would reject).
pub fn sanitise_relays(relays: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for r in relays {
        let r = r.trim().to_string();
        if !r.starts_with("wss://") || out.contains(&r) {
            continue;
        }
        out.push(r);
        if out.len() == 8 {
            break;
        }
    }
    out
}
```

(c) in `from_preset` (and anywhere else that constructs `GraphicsSettings`
field-by-field), add:

```rust
            online_relays: default_online_relays(),
            online_port: DEFAULT_ONLINE_PORT,
```

(d) in `clamp`, add:

```rust
        // An empty or all-rubbish relay list would make online play silently
        // impossible, so repair rather than accept it.
        self.online_relays = sanitise_relays(std::mem::take(&mut self.online_relays));
        if self.online_relays.is_empty() {
            self.online_relays = default_online_relays();
        }
```

(e) `preset()` derives from the quality dials only — **do not** add either field
to it (the test above pins this).

- [ ] **Step 4: Add the Settings → Online heading**

In `game/engine/src/menu.rs`, in the settings panel, after the last existing
section and inside a native gate, add:

```rust
    // Online play by contact (spec §6). Native only — there is no online play
    // on the web taster.
    #[cfg(not(target_arch = "wasm32"))]
    {
        ui.add_space(12.0);
        ui.label(egui::RichText::new("Online").size(15.0).strong());
        ui.label(
            egui::RichText::new(
                "Used only to set up a connection to a friend. Your game never runs \
                 through these.",
            )
            .size(11.0)
            .color(DIM_TEXT),
        );
        ui.add_space(4.0);
        let mut port = settings.online_port.to_string();
        ui.horizontal(|ui| {
            ui.label("Port");
            if ui
                .add(egui::TextEdit::singleline(&mut port).desired_width(70.0))
                .changed()
                && let Ok(p) = port.parse::<u16>()
            {
                settings.online_port = p;
                settings.save();
            }
            ui.label(
                egui::RichText::new("0 = pick one automatically")
                    .size(10.0)
                    .color(DIM_TEXT),
            );
        });
        ui.add_space(4.0);
        ui.label(egui::RichText::new("Relays (one per line)").size(12.0));
        let mut text = settings.online_relays.join("\n");
        if ui
            .add(egui::TextEdit::multiline(&mut text).desired_rows(4))
            .changed()
        {
            settings.online_relays = crate::graphics_settings::sanitise_relays(
                text.lines().map(str::to_string).collect(),
            );
            settings.save();
        }
    }
```

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo test --bin axenstax-engine graphics_settings 2>&1 | tail -20
```
Expected: PASS — the 6 new tests plus every pre-existing settings test.

- [ ] **Step 6: Verify the wasm build still compiles**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 trunk build 2>&1 | tail -10
```
Expected: `success` — `GraphicsSettings` is cross-platform, so the two fields
must compile on wasm; only the settings **UI block** is gated.

- [ ] **Step 7: Commit**

```bash
git add game/engine/src/graphics_settings.rs game/engine/src/menu.rs && git commit -m "$(cat <<'EOF'
feat(online): online_relays + online_port settings and the Settings→Online panel (P2)

serde(default) on both so an existing settings.json still loads; clamp() repairs
an empty relay list rather than leaving online play silently impossible.
Neither field enters preset detection — they are not GPU load. The panel says
plainly that relays are setup-only and never carry the game.

Claude-Session: https://claude.ai/code/session_011gW6q5QFBTbF1BNMHiayce
EOF
)" -- game/engine/src/graphics_settings.rs game/engine/src/menu.rs
```

---

## Phase 3 — Reachability

### Task 10: STUN codec (`nat/stun.rs`)

**Files:**
- Create: `game/engine/src/nat/mod.rs`
- Create: `game/engine/src/nat/stun.rs`
- Modify: `game/engine/src/main.rs`

**Interfaces:**
- Consumes: `std::net::UdpSocket`, `getrandom` (already a dependency).
- Produces:
  ```rust
  pub const MAGIC_COOKIE: u32 = 0x2112_A442;
  pub const BINDING_REQUEST: u16 = 0x0001;
  pub const BINDING_SUCCESS: u16 = 0x0101;
  pub const STUN_SERVERS: [&str; 2] = ["stun.l.google.com:19302", "stun.cloudflare.com:3478"];
  #[derive(Clone, Debug, PartialEq, Eq)]
  pub enum StunError { TooShort, BadCookie, NotSuccess(u16), TxidMismatch, NoXorMappedAddress, BadAttribute }
  pub fn random_txid() -> [u8; 12];
  pub fn encode_binding_request(txid: &[u8; 12]) -> Vec<u8>;
  pub fn parse_header(buf: &[u8]) -> Result<(u16, u16, [u8; 12]), StunError>;
  pub fn parse_binding_response(buf: &[u8], txid: &[u8; 12]) -> Result<std::net::SocketAddr, StunError>;
  pub fn reflexive_address(sock: &std::net::UdpSocket, server: &str, timeout: std::time::Duration)
      -> Result<std::net::SocketAddr, String>;
  ```

- [ ] **Step 1: Write the failing tests, with the RFC 5769 vectors verbatim**

Create `game/engine/src/nat/stun.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

    /// RFC 5769 §2.1 "Sample Request", byte for byte.
    ///
    /// ```text
    ///    00 01 00 58     Request type and message length
    ///    21 12 a4 42     Magic cookie
    ///    b7 e7 a7 01     }
    ///    bc 34 d6 86     |  Transaction ID
    ///    fa 87 df ae     }
    ///    80 22 00 10     SOFTWARE attribute header
    ///    53 54 55 4e     }
    ///    20 74 65 73     |  User-agent...
    ///    74 20 63 6c     |  ...name
    ///    69 65 6e 74     }
    ///    00 24 00 04     PRIORITY attribute header
    ///    6e 00 01 ff     ICE priority value
    ///    80 29 00 08     ICE-CONTROLLED attribute header
    ///    93 2f f9 b1     }  Pseudo-random tie breaker...
    ///    51 26 3b 36     }   ...for ICE control
    ///    00 06 00 09     USERNAME attribute header
    ///    65 76 74 6a     }
    ///    3a 68 36 76     |  Username value (9 bytes) and padding (3 bytes)
    ///    59 20 20 20     }
    ///    00 08 00 14     MESSAGE-INTEGRITY attribute header
    ///    9a ea a7 0c     }
    ///    bf d8 cb 56     |
    ///    78 1e f2 b5     |  HMAC-SHA1 fingerprint
    ///    b2 d3 f2 49     |
    ///    c1 b5 71 a2     }
    ///    80 28 00 04     FINGERPRINT attribute header
    ///    e5 7a 3b cf     CRC32 fingerprint
    /// ```
    const RFC5769_SAMPLE_REQUEST_HEX: &str = concat!(
        "000100582112a442b7e7a701bc34d686fa87dfae",
        "802200105354554e207465737420636c69656e74",
        "002400046e0001ff",
        "80290008932ff9b151263b36",
        "000600096576746a3a68367659202020",
        "000800149aeaa70cbfd8cb56781ef2b5b2d3f249c1b571a2",
        "80280004e57a3bcf",
    );

    /// RFC 5769 §2.2 "Sample IPv4 Response", byte for byte.
    ///
    /// ```text
    ///    01 01 00 3c     Response type and message length
    ///    21 12 a4 42     Magic cookie
    ///    b7 e7 a7 01     }
    ///    bc 34 d6 86     |  Transaction ID
    ///    fa 87 df ae     }
    ///    80 22 00 0b     SOFTWARE attribute header
    ///    74 65 73 74     }
    ///    20 76 65 63     |  UTF-8 server name
    ///    74 6f 72 20     }
    ///    00 20 00 08     XOR-MAPPED-ADDRESS attribute header
    ///    00 01 a1 47     Address family (IPv4) and xor'd mapped port number
    ///    e1 12 a6 43     Xor'd mapped IPv4 address
    ///    00 08 00 14     MESSAGE-INTEGRITY attribute header
    ///    2b 91 f5 99     }
    ///    fd 9e 90 c3     |
    ///    8c 74 89 f9     |  HMAC-SHA1 fingerprint
    ///    2a f9 ba 53     |
    ///    f0 6b e7 d7     }
    ///    80 28 00 04     FINGERPRINT attribute header
    ///    c0 7d 4c 96     CRC32 fingerprint
    /// ```
    ///
    /// The XOR-MAPPED-ADDRESS decodes to **192.0.2.1:32853**.
    const RFC5769_SAMPLE_IPV4_RESPONSE_HEX: &str = concat!(
        "0101003c2112a442b7e7a701bc34d686fa87dfae",
        "8022000b7465737420766563746f7220",
        "002000080001a147e112a643",
        "000800142b91f599fd9e90c38c7489f92af9ba53f06be7d7",
        "80280004c07d4c96",
    );

    /// The transaction ID both RFC 5769 samples use.
    const RFC5769_TXID: [u8; 12] = [
        0xb7, 0xe7, 0xa7, 0x01, 0xbc, 0x34, 0xd6, 0x86, 0xfa, 0x87, 0xdf, 0xae,
    ];

    fn bytes(hex_str: &str) -> Vec<u8> {
        hex::decode(hex_str).expect("test vector is valid hex")
    }

    #[test]
    fn rfc5769_sample_request_header_parses() {
        let buf = bytes(RFC5769_SAMPLE_REQUEST_HEX);
        assert_eq!(buf.len(), 108, "20-byte header + 88 bytes of attributes");
        let (msg_type, len, txid) = parse_header(&buf).unwrap();
        assert_eq!(msg_type, BINDING_REQUEST);
        assert_eq!(len, 0x58);
        assert_eq!(txid, RFC5769_TXID);
        assert_eq!(len as usize, buf.len() - 20, "length covers the attributes");
    }

    #[test]
    fn rfc5769_sample_request_is_not_a_success_response() {
        let buf = bytes(RFC5769_SAMPLE_REQUEST_HEX);
        assert_eq!(
            parse_binding_response(&buf, &RFC5769_TXID),
            Err(StunError::NotSuccess(BINDING_REQUEST))
        );
    }

    #[test]
    fn rfc5769_sample_ipv4_response_yields_192_0_2_1_port_32853() {
        let buf = bytes(RFC5769_SAMPLE_IPV4_RESPONSE_HEX);
        assert_eq!(buf.len(), 80, "20-byte header + 60 bytes of attributes");
        let addr = parse_binding_response(&buf, &RFC5769_TXID).unwrap();
        assert_eq!(addr, SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)), 32853));
    }

    #[test]
    fn a_response_for_another_transaction_is_refused() {
        let buf = bytes(RFC5769_SAMPLE_IPV4_RESPONSE_HEX);
        assert_eq!(
            parse_binding_response(&buf, &[0u8; 12]),
            Err(StunError::TxidMismatch)
        );
    }

    #[test]
    fn a_response_with_a_wrong_magic_cookie_is_refused() {
        let mut buf = bytes(RFC5769_SAMPLE_IPV4_RESPONSE_HEX);
        buf[4] = 0x00;
        assert_eq!(parse_binding_response(&buf, &RFC5769_TXID), Err(StunError::BadCookie));
    }

    #[test]
    fn a_truncated_datagram_is_refused_without_panicking() {
        let full = bytes(RFC5769_SAMPLE_IPV4_RESPONSE_HEX);
        for cut in 0..20 {
            assert_eq!(parse_header(&full[..cut]), Err(StunError::TooShort));
        }
        // Truncated mid-attribute: the parser must stop, not read past the end.
        assert!(parse_binding_response(&full[..30], &RFC5769_TXID).is_err());
    }

    #[test]
    fn a_success_response_without_xor_mapped_address_is_refused() {
        // Header only, length 0 — a well-formed but useless success response.
        let mut buf = Vec::new();
        buf.extend_from_slice(&BINDING_SUCCESS.to_be_bytes());
        buf.extend_from_slice(&0u16.to_be_bytes());
        buf.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
        buf.extend_from_slice(&RFC5769_TXID);
        assert_eq!(
            parse_binding_response(&buf, &RFC5769_TXID),
            Err(StunError::NoXorMappedAddress)
        );
    }

    #[test]
    fn an_ipv6_xor_mapped_address_decodes() {
        // Built by XOR-ing a known address with cookie‖txid here in the test,
        // so this asserts the parser's arithmetic rather than a hand-copied
        // constant. The IPv4 path is pinned to the real RFC vector above.
        let want_ip = Ipv6Addr::new(0x2001, 0xdb8, 0x1234, 0x5678, 0x11, 0x2233, 0x4455, 0x6677);
        let want_port: u16 = 32853;

        let mut key = [0u8; 16];
        key[..4].copy_from_slice(&MAGIC_COOKIE.to_be_bytes());
        key[4..].copy_from_slice(&RFC5769_TXID);
        let mut xored = want_ip.octets();
        for (b, k) in xored.iter_mut().zip(key.iter()) {
            *b ^= k;
        }
        let xport = want_port ^ (MAGIC_COOKIE >> 16) as u16;

        let mut attr = vec![0x00, 0x02]; // reserved byte + family IPv6
        attr.extend_from_slice(&xport.to_be_bytes());
        attr.extend_from_slice(&xored);

        let mut buf = Vec::new();
        buf.extend_from_slice(&BINDING_SUCCESS.to_be_bytes());
        buf.extend_from_slice(&((4 + attr.len()) as u16).to_be_bytes());
        buf.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
        buf.extend_from_slice(&RFC5769_TXID);
        buf.extend_from_slice(&0x0020u16.to_be_bytes());
        buf.extend_from_slice(&(attr.len() as u16).to_be_bytes());
        buf.extend_from_slice(&attr);

        assert_eq!(
            parse_binding_response(&buf, &RFC5769_TXID).unwrap(),
            SocketAddr::new(IpAddr::V6(want_ip), want_port)
        );
    }

    #[test]
    fn our_binding_request_is_a_bare_20_byte_header() {
        let txid = random_txid();
        let req = encode_binding_request(&txid);
        assert_eq!(req.len(), 20, "no attributes: type, length 0, cookie, txid");
        let (msg_type, len, got) = parse_header(&req).unwrap();
        assert_eq!(msg_type, BINDING_REQUEST);
        assert_eq!(len, 0);
        assert_eq!(got, txid);
        assert_eq!(&req[4..8], &MAGIC_COOKIE.to_be_bytes());
    }

    #[test]
    fn transaction_ids_differ_between_requests() {
        assert_ne!(random_txid(), random_txid());
    }
}
```

- [ ] **Step 2: Run them to verify they fail**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo test --bin axenstax-engine nat::stun 2>&1 | tail -20
```
Expected: FAIL — `file not found for module nat`.

- [ ] **Step 3: Write the implementation**

Create `game/engine/src/nat/mod.rs`:

```rust
//! Getting two home machines to talk directly.
//!
//! Everything here is about **addresses**, not about the game: gather the
//! addresses this machine might be reachable at, punch a hole through the
//! router toward the peer's, and race a QUIC connect across all of them. The
//! game traffic that follows is the existing transport, unchanged.
//!
//! Spec: `docs/superpowers/specs/2026-09-06-online-play-by-contact-design.md` §4.
#![cfg(not(target_arch = "wasm32"))]

pub mod candidates;
pub mod punch;
pub mod stun;
pub mod upnp;
```

Create `game/engine/src/nat/stun.rs` (above the test module):

```rust
//! A minimal RFC 5389 STUN client: send a Binding Request, read
//! XOR-MAPPED-ADDRESS out of the Binding Success Response.
//!
//! Hand-rolled — about 120 lines against a stable 2008 wire format, versus a
//! dependency whose async runtime and ICE machinery we would not use. The codec
//! is pure and pinned to the RFC 5769 test vectors; only `reflexive_address`
//! touches a socket.
//!
//! **The request goes out on the socket quinn will later own**, so the mapping
//! the STUN server reports is the one QUIC traffic will actually use. Non-QUIC
//! datagrams arriving on a quinn socket are dropped, which is why this runs
//! *before* the endpoint is created.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};
use std::time::Duration;

/// RFC 5389 §6. Present in every message; also what distinguishes STUN from
/// the older RFC 3489 framing.
pub const MAGIC_COOKIE: u32 = 0x2112_A442;
pub const BINDING_REQUEST: u16 = 0x0001;
pub const BINDING_SUCCESS: u16 = 0x0101;
const ATTR_XOR_MAPPED_ADDRESS: u16 = 0x0020;
const FAMILY_IPV4: u8 = 0x01;
const FAMILY_IPV6: u8 = 0x02;
const HEADER_LEN: usize = 20;

/// Public STUN servers used to learn this machine's reflexive address. Two, so
/// one being down is not the end of it. Neither is ours and neither ever sees
/// game traffic.
pub const STUN_SERVERS: [&str; 2] = ["stun.l.google.com:19302", "stun.cloudflare.com:3478"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StunError {
    TooShort,
    BadCookie,
    /// A well-formed STUN message that is not a Binding Success Response.
    NotSuccess(u16),
    TxidMismatch,
    NoXorMappedAddress,
    /// An attribute ran past the end of the buffer, or had an impossible length.
    BadAttribute,
}

/// 96 bits of transaction id, as RFC 5389 §6 requires.
pub fn random_txid() -> [u8; 12] {
    let mut t = [0u8; 12];
    getrandom::fill(&mut t).expect("OS RNG unavailable");
    t
}

/// A Binding Request with no attributes — all we need, and all we send.
pub fn encode_binding_request(txid: &[u8; 12]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(HEADER_LEN);
    buf.extend_from_slice(&BINDING_REQUEST.to_be_bytes());
    buf.extend_from_slice(&0u16.to_be_bytes()); // no attributes
    buf.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
    buf.extend_from_slice(txid);
    buf
}

/// `(message type, attribute length, transaction id)`.
pub fn parse_header(buf: &[u8]) -> Result<(u16, u16, [u8; 12]), StunError> {
    if buf.len() < HEADER_LEN {
        return Err(StunError::TooShort);
    }
    let msg_type = u16::from_be_bytes([buf[0], buf[1]]);
    let len = u16::from_be_bytes([buf[2], buf[3]]);
    let cookie = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
    if cookie != MAGIC_COOKIE {
        return Err(StunError::BadCookie);
    }
    let mut txid = [0u8; 12];
    txid.copy_from_slice(&buf[8..20]);
    Ok((msg_type, len, txid))
}

/// Decode one XOR-MAPPED-ADDRESS value (RFC 5389 §15.2): the port is XORed
/// with the top 16 bits of the cookie, an IPv4 address with the cookie, an IPv6
/// address with cookie‖txid.
fn decode_xor_mapped(value: &[u8], txid: &[u8; 12]) -> Result<SocketAddr, StunError> {
    if value.len() < 4 {
        return Err(StunError::BadAttribute);
    }
    let family = value[1];
    let xport = u16::from_be_bytes([value[2], value[3]]);
    let port = xport ^ (MAGIC_COOKIE >> 16) as u16;
    let cookie = MAGIC_COOKIE.to_be_bytes();
    match family {
        FAMILY_IPV4 => {
            if value.len() < 8 {
                return Err(StunError::BadAttribute);
            }
            let mut o = [0u8; 4];
            for i in 0..4 {
                o[i] = value[4 + i] ^ cookie[i];
            }
            Ok(SocketAddr::new(IpAddr::V4(Ipv4Addr::from(o)), port))
        }
        FAMILY_IPV6 => {
            if value.len() < 20 {
                return Err(StunError::BadAttribute);
            }
            let mut key = [0u8; 16];
            key[..4].copy_from_slice(&cookie);
            key[4..].copy_from_slice(txid);
            let mut o = [0u8; 16];
            for i in 0..16 {
                o[i] = value[4 + i] ^ key[i];
            }
            Ok(SocketAddr::new(IpAddr::V6(Ipv6Addr::from(o)), port))
        }
        _ => Err(StunError::BadAttribute),
    }
}

/// Pull the reflexive address out of a Binding Success Response.
///
/// Attributes we don't use (SOFTWARE, MESSAGE-INTEGRITY, FINGERPRINT) are
/// skipped, which is what makes the RFC 5769 vectors — full of them — parse.
pub fn parse_binding_response(buf: &[u8], txid: &[u8; 12]) -> Result<SocketAddr, StunError> {
    let (msg_type, attr_len, got_txid) = parse_header(buf)?;
    if msg_type != BINDING_SUCCESS {
        return Err(StunError::NotSuccess(msg_type));
    }
    if &got_txid != txid {
        return Err(StunError::TxidMismatch);
    }
    let end = HEADER_LEN
        .checked_add(attr_len as usize)
        .filter(|e| *e <= buf.len())
        .ok_or(StunError::TooShort)?;

    let mut i = HEADER_LEN;
    while i + 4 <= end {
        let a_type = u16::from_be_bytes([buf[i], buf[i + 1]]);
        let a_len = u16::from_be_bytes([buf[i + 2], buf[i + 3]]) as usize;
        let value_start = i + 4;
        let value_end = value_start.checked_add(a_len).ok_or(StunError::BadAttribute)?;
        if value_end > end {
            return Err(StunError::BadAttribute);
        }
        if a_type == ATTR_XOR_MAPPED_ADDRESS {
            return decode_xor_mapped(&buf[value_start..value_end], txid);
        }
        // Attributes are padded to a 4-byte boundary.
        i = value_end + ((4 - (a_len % 4)) % 4);
    }
    Err(StunError::NoXorMappedAddress)
}

/// OWNER BOUNDARY (needs the internet). Ask one STUN server what address this
/// socket appears to come from. Blocking, with `timeout` as the read deadline;
/// restores the socket's previous read timeout before returning.
pub fn reflexive_address(
    sock: &UdpSocket,
    server: &str,
    timeout: Duration,
) -> Result<SocketAddr, String> {
    let txid = random_txid();
    let req = encode_binding_request(&txid);
    sock.send_to(&req, server)
        .map_err(|e| format!("stun send to {server}: {e}"))?;
    let previous = sock.read_timeout().ok().flatten();
    sock.set_read_timeout(Some(timeout))
        .map_err(|e| format!("stun set timeout: {e}"))?;

    let mut buf = [0u8; 1500];
    let result = loop {
        match sock.recv_from(&mut buf) {
            Ok((n, _from)) => match parse_binding_response(&buf[..n], &txid) {
                Ok(addr) => break Ok(addr),
                // Not our transaction (or not STUN at all) — keep reading until
                // the deadline rather than giving up on the first stray packet.
                Err(_) => continue,
            },
            Err(e) => break Err(format!("stun recv from {server}: {e}")),
        }
    };
    let _ = sock.set_read_timeout(previous);
    result
}
```

- [ ] **Step 4: Register the module**

In `game/engine/src/main.rs`:

```rust
// Online play by contact — NAT traversal (STUN, candidates, UPnP, punching).
#[cfg(not(target_arch = "wasm32"))]
mod nat;
```

Create placeholder `game/engine/src/nat/{candidates,upnp,punch}.rs`, each
containing only:

```rust
//! Filled in by the next task.
#![cfg(not(target_arch = "wasm32"))]
```

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo test --bin axenstax-engine nat::stun 2>&1 | tail -20
```
Expected: PASS — 10 passed. The two RFC vectors are the load-bearing ones:
`rfc5769_sample_ipv4_response_yields_192_0_2_1_port_32853` must produce exactly
`192.0.2.1:32853`.

- [ ] **Step 6: Commit**

```bash
git add game/engine/src/nat game/engine/src/main.rs && git commit -m "$(cat <<'EOF'
feat(online): RFC 5389 STUN codec, pinned to the RFC 5769 vectors (P3)

Hand-rolled Binding Request/Response with XOR-MAPPED-ADDRESS decode for v4 and
v6. The §2.1 request and §2.2 IPv4 response vectors are pasted byte for byte and
the v4 one must decode to 192.0.2.1:32853; the v6 path is checked by XOR-ing a
known address in the test rather than by a hand-copied constant. Unknown
attributes are skipped, which is what makes the real vectors parse.

Claude-Session: https://claude.ai/code/session_011gW6q5QFBTbF1BNMHiayce
EOF
)" -- game/engine/src/nat game/engine/src/main.rs
```

---

### Task 11: Candidate gathering and ordering (`nat/candidates.rs`)

**Files:**
- Replace: `game/engine/src/nat/candidates.rs`

**Interfaces:**
- Consumes: `nat::stun::{reflexive_address, STUN_SERVERS}` (Task 10),
  `rendezvous::payload::Candidate` (Task 6). **Nothing from Task 12** — `gather`
  takes an already-obtained UPnP mapping as a plain `Option<SocketAddr>`
  parameter rather than calling `upnp::map_port` itself, because the caller has
  to own that mapping anyway (it must be renewed hourly and released on stop).
  So this task compiles and its tests pass before Task 12 exists.
- Produces:
  ```rust
  #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
  pub enum CandidateKind { Lan, V6, Upnp, Stun }
  impl CandidateKind { pub fn as_str(self) -> &'static str; pub fn parse(s: &str) -> Option<Self>; }
  pub fn sort_candidates(list: Vec<Candidate>) -> Vec<Candidate>;
  pub fn reachable_beyond_lan(list: &[Candidate]) -> bool;
  pub fn parse_addrs(list: &[Candidate]) -> Vec<std::net::SocketAddr>;
  pub fn local_outbound_v4() -> Option<std::net::Ipv4Addr>;
  pub fn local_outbound_v6() -> Option<std::net::Ipv6Addr>;
  pub fn gather(sock: &std::net::UdpSocket, upnp: Option<std::net::SocketAddr>,
                stun_timeout: std::time::Duration) -> Vec<Candidate>;
  ```

- [ ] **Step 1: Write the failing tests**

Replace `game/engine/src/nat/candidates.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;

    fn c(kind: &str, addr: &str) -> Candidate {
        Candidate { kind: kind.to_string(), addr: addr.to_string() }
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
```

- [ ] **Step 2: Run them to verify they fail**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo test --bin axenstax-engine nat::candidates 2>&1 | tail -20
```
Expected: FAIL — `cannot find type CandidateKind`.

- [ ] **Step 3: Write the implementation**

Put this above the test module in `game/engine/src/nat/candidates.rs`:

```rust
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

/// The dialable addresses, in order. Unparseable entries are skipped — one
/// garbled candidate must not cost the peer the rest.
pub fn parse_addrs(list: &[Candidate]) -> Vec<SocketAddr> {
    list.iter()
        .filter_map(|c| c.addr.parse::<SocketAddr>().ok())
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
        IpAddr::V4(v4) if !v4.is_loopback() && !v4.is_unspecified() => Some(v4),
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
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo test --bin axenstax-engine nat::candidates 2>&1 | tail -20
```
Expected: PASS — 9 passed. (`gather` runs on a loopback socket with a 1 ms STUN
budget, so it needs no internet; the STUN attempts simply time out.)

- [ ] **Step 5: Commit**

```bash
git add game/engine/src/nat/candidates.rs && git commit -m "$(cat <<'EOF'
feat(online): candidate gathering + priority ordering (P3)

Four kinds on ONE already-bound socket, so the port a peer is told to dial is
the port QUIC answers on. Declaration order is priority order (lan, v6, upnp,
stun) and an unrecognised future kind sorts last rather than being dropped.
Outbound IP is found by the routing-table trick, which picks the right
interface on a multi-homed box; link-local and unique-local v6 are rejected.

Claude-Session: https://claude.ai/code/session_011gW6q5QFBTbF1BNMHiayce
EOF
)" -- game/engine/src/nat/candidates.rs
```

---

### Task 12: UPnP port mapping (`nat/upnp.rs`)

**Files:**
- Replace: `game/engine/src/nat/upnp.rs`
- Modify: `game/engine/Cargo.toml` (the one new crate)

**Interfaces:**
- Consumes (igd-next 0.17.1, blocking API — exact signatures):
  ```rust
  pub fn search_gateway(options: SearchOptions) -> Result<Gateway, SearchError>;
  pub struct SearchOptions { pub bind_addr: SocketAddr, pub broadcast_address: SocketAddr,
                             pub timeout: Option<Duration>, pub single_search_timeout: Option<Duration> }
  impl Gateway {
      pub fn get_external_ip(&self) -> Result<IpAddr, GetExternalIpError>;
      pub fn add_any_port(&self, protocol: PortMappingProtocol, local_addr: SocketAddr,
                          lease_duration: u32, description: &str) -> Result<u16, AddAnyPortError>;
      pub fn add_port(&self, protocol: PortMappingProtocol, external_port: u16,
                      local_addr: SocketAddr, lease_duration: u32, description: &str)
                      -> Result<(), AddPortError>;
      pub fn remove_port(&self, protocol: PortMappingProtocol, external_port: u16)
                      -> Result<(), RemovePortError>;
  }
  pub enum PortMappingProtocol { TCP, UDP }
  ```
- Produces:
  ```rust
  pub const LEASE_SECS: u32 = 7200;
  pub const RENEW_EVERY: std::time::Duration = std::time::Duration::from_secs(3600);
  pub const SEARCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);
  pub const MAPPING_DESCRIPTION: &str = "AxeNStax";
  pub fn renew_due(last_renew: std::time::Instant, now: std::time::Instant) -> bool;
  pub struct PortMapping { /* private */ }
  impl PortMapping {
      pub fn external_addr(&self) -> std::net::SocketAddr;
      pub fn renew(&mut self, now: std::time::Instant) -> Result<(), String>;
      pub fn remove(self);
  }
  pub fn map_port(local: std::net::SocketAddr, search_timeout: std::time::Duration)
      -> Result<PortMapping, String>;
  ```

- [ ] **Step 1: Add the dependency**

In `game/engine/Cargo.toml`, under `[target.'cfg(not(target_arch = "wasm32"))'.dependencies]`:

```toml
# UPnP IGD port mapping so a home router forwards the online-play UDP port
# without anyone editing router settings (spec §4.1). Blocking API used on a
# worker thread — the `aio_tokio` feature is deliberately OFF, so this pulls no
# async stack of its own. NATIVE-ONLY: there is no router to ask from a browser.
igd-next = "0.17.1"
```

Then fetch it (network step, done once):

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo fetch 2>&1 | tail -5
```
Expected: `igd-next v0.17.1` downloaded (or already cached).

- [ ] **Step 2: Write the failing tests**

Replace `game/engine/src/nat/upnp.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn renewal_is_due_after_an_hour_and_not_before() {
        let t0 = Instant::now();
        assert!(!renew_due(t0, t0));
        assert!(!renew_due(t0, t0 + RENEW_EVERY - Duration::from_secs(1)));
        assert!(renew_due(t0, t0 + RENEW_EVERY));
        assert!(renew_due(t0, t0 + RENEW_EVERY + Duration::from_secs(1)));
    }

    #[test]
    fn the_lease_outlives_two_renewal_periods() {
        // If the lease were shorter than the renewal interval, a mapping would
        // lapse between renewals and friends would drop out for no visible
        // reason. Two periods of headroom absorbs a missed renewal.
        assert!(
            u64::from(LEASE_SECS) >= 2 * RENEW_EVERY.as_secs(),
            "lease {LEASE_SECS}s must cover two {}s renewal periods",
            RENEW_EVERY.as_secs()
        );
    }

    #[test]
    fn the_mapping_description_names_the_game_and_nothing_private() {
        // This string shows up in the router's admin page, where anyone on the
        // network can read it. It must not carry a player name, npub or world.
        assert_eq!(MAPPING_DESCRIPTION, "AxeNStax");
        assert!(!MAPPING_DESCRIPTION.contains("npub"));
    }

    /// OWNER BOUNDARY (needs a real IGD router) — not run in CI.
    /// Run manually on a home network:
    /// `cargo test --bin axenstax-engine nat::upnp -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_router_accepts_and_releases_a_mapping() {
        let sock = std::net::UdpSocket::bind("0.0.0.0:0").unwrap();
        let local = std::net::SocketAddr::new(
            std::net::IpAddr::V4(crate::nat::candidates::local_outbound_v4().unwrap()),
            sock.local_addr().unwrap().port(),
        );
        let m = map_port(local, SEARCH_TIMEOUT).expect("router should accept a UDP mapping");
        println!("mapped to {}", m.external_addr());
        m.remove();
    }
}
```

- [ ] **Step 3: Run them to verify they fail**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo test --bin axenstax-engine nat::upnp 2>&1 | tail -20
```
Expected: FAIL — `cannot find function renew_due`.

- [ ] **Step 4: Write the implementation**

Put this above the test module in `game/engine/src/nat/upnp.rs`:

```rust
//! Asking the home router to forward the online-play UDP port.
//!
//! Most consumer routers speak UPnP IGD and will agree to this without anyone
//! opening a router admin page — which is the whole point: "no port forwarding
//! for most homes" (spec §0). A router that refuses is not an error, it is one
//! fewer candidate; the joiner still has IPv6 and STUN to try, and the host is
//! told plainly if none of them worked.
//!
//! The lease is **finite and renewed**, never infinite: a game that leaves a
//! permanent hole in somebody's router after it exits is not a good guest.
//! `PortMapping::remove` is called when hosting stops.
//!
//! Blocking API on purpose — this runs on a worker thread during the ≤3 s
//! candidate-gathering budget, so `igd-next`'s `aio_tokio` feature is off and
//! no async stack comes with it.
#![cfg(not(target_arch = "wasm32"))]

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use igd_next::{search_gateway, Gateway, PortMappingProtocol, SearchOptions};

/// Lease length asked of the router. Two hours: long enough that a missed
/// renewal is survivable, short enough that a crashed game's mapping expires by
/// itself.
pub const LEASE_SECS: u32 = 7200;
/// How often a live mapping is re-asserted while hosting.
pub const RENEW_EVERY: Duration = Duration::from_secs(3600);
/// How long to wait for a router to answer the SSDP search. Kept tight — this
/// sits inside the candidate-gathering budget, and a router that hasn't
/// answered in two seconds is not going to.
pub const SEARCH_TIMEOUT: Duration = Duration::from_secs(2);
/// What the router's admin page will show. Deliberately just the game's name:
/// no player name, no npub, no world (red line 3 — and anyone on the network
/// can read it).
pub const MAPPING_DESCRIPTION: &str = "AxeNStax";

/// Whether a mapping made/renewed at `last_renew` is due again at `now`. Pure,
/// so the schedule is testable without a router.
pub fn renew_due(last_renew: Instant, now: Instant) -> bool {
    now.duration_since(last_renew) >= RENEW_EVERY
}

/// A live UDP port mapping on the home router.
pub struct PortMapping {
    gateway: Gateway,
    external: SocketAddr,
    local: SocketAddr,
    last_renew: Instant,
}

impl PortMapping {
    /// The address a peer outside the house should dial.
    pub fn external_addr(&self) -> SocketAddr {
        self.external
    }

    /// Re-assert the mapping. No-op until [`renew_due`]; call it on a timer
    /// while hosting.
    pub fn renew(&mut self, now: Instant) -> Result<(), String> {
        if !renew_due(self.last_renew, now) {
            return Ok(());
        }
        self.gateway
            .add_port(
                PortMappingProtocol::UDP,
                self.external.port(),
                self.local,
                LEASE_SECS,
                MAPPING_DESCRIPTION,
            )
            .map_err(|e| format!("upnp renew: {e}"))?;
        self.last_renew = now;
        Ok(())
    }

    /// Give the port back. Best-effort: a router that has already forgotten the
    /// mapping (rebooted, lease lapsed) is not a failure worth surfacing.
    pub fn remove(self) {
        if let Err(e) = self
            .gateway
            .remove_port(PortMappingProtocol::UDP, self.external.port())
        {
            log::debug!("[nat] releasing the port mapping: {e}");
        }
    }
}

/// Ask the router to forward some external UDP port to `local`.
///
/// `add_any_port` lets the router pick, which succeeds far more often than
/// demanding a specific one (the port may already be claimed, and some firmware
/// refuses `AddPortMapping` outright while accepting `AddAnyPortMapping`).
pub fn map_port(local: SocketAddr, search_timeout: Duration) -> Result<PortMapping, String> {
    let gateway = search_gateway(SearchOptions {
        timeout: Some(search_timeout),
        ..Default::default()
    })
    .map_err(|e| format!("no UPnP router found: {e}"))?;

    let external_ip = gateway
        .get_external_ip()
        .map_err(|e| format!("router would not report its external address: {e}"))?;

    let external_port = gateway
        .add_any_port(
            PortMappingProtocol::UDP,
            local,
            LEASE_SECS,
            MAPPING_DESCRIPTION,
        )
        .map_err(|e| format!("router refused a port mapping: {e}"))?;

    Ok(PortMapping {
        gateway,
        external: SocketAddr::new(external_ip, external_port),
        local,
        last_renew: Instant::now(),
    })
}
```

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo test --bin axenstax-engine nat::upnp 2>&1 | tail -20
```
Expected: PASS — 3 passed, 1 ignored (the live-router one).

- [ ] **Step 6: Verify the wasm build is unaffected**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 trunk build 2>&1 | tail -10
```
Expected: `success`. `igd-next` is declared under the native target only, so it
must not appear in the wasm dependency graph at all.

- [ ] **Step 7: Commit**

```bash
git add game/engine/Cargo.toml game/engine/Cargo.lock game/engine/src/nat/upnp.rs && git commit -m "$(cat <<'EOF'
feat(online): UPnP IGD port mapping (P3)

The one new crate: igd-next 0.17.1, blocking API, aio_tokio deliberately off so
it brings no async stack. add_any_port (the router picks) succeeds far more
often than demanding a port. The lease is finite (2h) and renewed hourly, and
removed when hosting stops — a game must not leave a permanent hole in
somebody's router. A router that refuses is one fewer candidate, not an error.

Claude-Session: https://claude.ai/code/session_011gW6q5QFBTbF1BNMHiayce
EOF
)" -- game/engine/Cargo.toml game/engine/Cargo.lock game/engine/src/nat/upnp.rs
```

---

### Task 13: Punch datagrams + the connect-race state machine (`nat/punch.rs`)

**Files:**
- Replace: `game/engine/src/nat/punch.rs`

**Interfaces:**
- Consumes: `std::net::UdpSocket`.
- Produces:
  ```rust
  pub const PUNCH_MAGIC: &[u8; 10] = b"AXNS-PUNCH";
  pub const PUNCH_COUNT: usize = 3;
  pub const PUNCH_GAP: std::time::Duration = std::time::Duration::from_millis(100);
  pub const CONNECT_STAGGER: std::time::Duration = std::time::Duration::from_millis(150);
  pub const CONNECT_DEADLINE: std::time::Duration = std::time::Duration::from_secs(8);

  pub fn punch_datagram(session: &str) -> Vec<u8>;
  pub fn is_punch(buf: &[u8]) -> Option<String>;
  pub fn send_punches(sock: &std::net::UdpSocket, targets: &[std::net::SocketAddr], session: &str);

  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  pub enum RaceOutcome { Won(usize), AllFailed, TimedOut }
  pub struct ConnectRace { /* private */ }
  impl ConnectRace {
      pub fn new(n: usize) -> Self;
      pub fn advance(&mut self, elapsed: std::time::Duration) -> Vec<usize>;
      pub fn on_connected(&mut self, idx: usize) -> Vec<usize>;
      pub fn on_failed(&mut self, idx: usize);
      pub fn outcome(&self) -> Option<RaceOutcome>;
  }
  ```

- [ ] **Step 1: Write the failing tests**

Replace `game/engine/src/nat/punch.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const S: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn a_punch_datagram_is_recognisable_and_carries_its_session() {
        let d = punch_datagram(S);
        assert!(d.starts_with(PUNCH_MAGIC));
        assert_eq!(is_punch(&d).as_deref(), Some(S));
    }

    #[test]
    fn anything_else_is_not_a_punch() {
        assert_eq!(is_punch(b"hello"), None);
        assert_eq!(is_punch(&[]), None);
        assert_eq!(is_punch(b"AXNS-PUNC"), None, "a truncated magic is not a match");
        // A QUIC Initial-looking datagram must never be read as a punch.
        assert_eq!(is_punch(&[0xc0, 0x00, 0x00, 0x00, 0x01]), None);
    }

    #[test]
    fn a_punch_with_a_non_utf8_tail_is_rejected_rather_than_lossy_decoded() {
        let mut d = PUNCH_MAGIC.to_vec();
        d.extend_from_slice(&[0xff, 0xfe]);
        assert_eq!(is_punch(&d), None);
    }

    #[test]
    fn dials_are_staggered_one_per_150ms_in_priority_order() {
        let mut r = ConnectRace::new(3);
        assert_eq!(r.advance(Duration::ZERO), vec![0], "the best candidate goes first");
        assert_eq!(r.advance(Duration::from_millis(149)), Vec::<usize>::new());
        assert_eq!(r.advance(Duration::from_millis(150)), vec![1]);
        assert_eq!(r.advance(Duration::from_millis(300)), vec![2]);
        assert_eq!(r.advance(Duration::from_millis(450)), Vec::<usize>::new());
        assert!(r.outcome().is_none(), "still racing");
    }

    #[test]
    fn a_late_advance_catches_up_every_due_dial_at_once() {
        // The caller may be a tick behind; nothing should be skipped.
        let mut r = ConnectRace::new(4);
        assert_eq!(r.advance(Duration::from_millis(500)), vec![0, 1, 2, 3]);
    }

    #[test]
    fn the_first_success_wins_and_the_others_are_aborted() {
        let mut r = ConnectRace::new(3);
        r.advance(Duration::from_millis(500));
        assert_eq!(r.on_connected(1), vec![0, 2], "every OTHER in-flight dial is aborted");
        assert_eq!(r.outcome(), Some(RaceOutcome::Won(1)));
    }

    #[test]
    fn a_second_success_after_a_winner_is_ignored() {
        // Two candidates can complete within microseconds of each other; only
        // one connection may be handed to the game.
        let mut r = ConnectRace::new(3);
        r.advance(Duration::from_millis(500));
        r.on_connected(1);
        assert_eq!(r.on_connected(2), Vec::<usize>::new(), "no second abort list");
        assert_eq!(r.outcome(), Some(RaceOutcome::Won(1)), "the first winner stands");
    }

    #[test]
    fn a_failed_candidate_does_not_end_the_race_until_all_have_failed() {
        let mut r = ConnectRace::new(2);
        r.advance(Duration::from_millis(500));
        r.on_failed(0);
        assert!(r.outcome().is_none(), "one left to try");
        r.on_failed(1);
        assert_eq!(r.outcome(), Some(RaceOutcome::AllFailed));
    }

    #[test]
    fn a_failure_after_a_win_cannot_turn_a_win_into_a_loss() {
        let mut r = ConnectRace::new(2);
        r.advance(Duration::from_millis(500));
        r.on_connected(0);
        r.on_failed(1);
        assert_eq!(r.outcome(), Some(RaceOutcome::Won(0)));
    }

    #[test]
    fn the_deadline_fires_at_eight_seconds() {
        let mut r = ConnectRace::new(2);
        r.advance(Duration::from_millis(500));
        assert!(r.outcome().is_none());
        r.advance(CONNECT_DEADLINE);
        assert_eq!(r.outcome(), Some(RaceOutcome::TimedOut));
        assert_eq!(
            r.advance(CONNECT_DEADLINE + Duration::from_secs(1)),
            Vec::<usize>::new(),
            "a finished race dials nothing more"
        );
    }

    #[test]
    fn a_race_with_no_candidates_fails_immediately() {
        let mut r = ConnectRace::new(0);
        assert_eq!(r.advance(Duration::ZERO), Vec::<usize>::new());
        assert_eq!(r.outcome(), Some(RaceOutcome::AllFailed));
    }

    #[test]
    fn send_punches_writes_three_datagrams_per_target() {
        // Two loopback sockets stand in for two houses. No timing assertion —
        // just that every target receives PUNCH_COUNT datagrams carrying the
        // session.
        let sender = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let receiver = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        receiver
            .set_read_timeout(Some(Duration::from_millis(500)))
            .unwrap();
        let target = receiver.local_addr().unwrap();

        send_punches(&sender, &[target], S);

        let mut buf = [0u8; 256];
        for i in 0..PUNCH_COUNT {
            let (n, _) = receiver.recv_from(&mut buf).expect("punch {i} should arrive");
            assert_eq!(is_punch(&buf[..n]).as_deref(), Some(S));
        }
    }
}
```

- [ ] **Step 2: Run them to verify they fail**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo test --bin axenstax-engine nat::punch 2>&1 | tail -20
```
Expected: FAIL — `cannot find function punch_datagram`.

- [ ] **Step 3: Write the implementation**

Put this above the test module in `game/engine/src/nat/punch.rs`:

```rust
//! Opening the hole, and racing across it.
//!
//! **The punch.** A home router will forward an inbound packet from an address
//! it has recently seen an outbound packet *to*. So both sides send a few
//! throwaway datagrams at each of the peer's candidate addresses before either
//! tries to connect. Nothing is expected back — the datagrams exist to teach
//! each router that this conversation is wanted.
//!
//! Punch datagrams are plain and unauthenticated on purpose: they carry no
//! secret, and forging one achieves nothing an attacker could not achieve by
//! sending any UDP packet. They arrive on the socket quinn owns, which drops
//! them as not-QUIC.
//!
//! **The race.** There is no way to know in advance which candidate will work,
//! so all of them are dialled, staggered 150 ms apart in priority order, and
//! the first to complete the ALPN handshake wins. [`ConnectRace`] is that
//! decision as a pure state machine — no sockets, no clock of its own — so
//! "first success wins, everything else is aborted, and there is never a second
//! winner" is provable in a unit test rather than hoped for at a playtest.
#![cfg(not(target_arch = "wasm32"))]

use std::net::{SocketAddr, UdpSocket};
use std::time::Duration;

/// Prefix that marks a datagram as ours. Chosen so it cannot collide with a
/// QUIC long header (whose first byte always has the high bit set).
pub const PUNCH_MAGIC: &[u8; 10] = b"AXNS-PUNCH";
/// How many punches per target. Three is enough to survive ordinary loss
/// without looking like a flood to anybody's router.
pub const PUNCH_COUNT: usize = 3;
pub const PUNCH_GAP: Duration = Duration::from_millis(100);
/// Gap between successive connect attempts, best candidate first.
pub const CONNECT_STAGGER: Duration = Duration::from_millis(150);
/// Overall budget for the connect race. Past this the joiner shows the
/// "couldn't reach" copy.
pub const CONNECT_DEADLINE: Duration = Duration::from_secs(8);

/// `AXNS-PUNCH` followed by the session id, so a stray punch from an unrelated
/// attempt is recognisable in a log.
pub fn punch_datagram(session: &str) -> Vec<u8> {
    let mut d = PUNCH_MAGIC.to_vec();
    d.extend_from_slice(session.as_bytes());
    d
}

/// The session id in a punch datagram, or `None` if this isn't one. A non-UTF-8
/// tail is rejected rather than lossily decoded — it is not a punch we sent.
pub fn is_punch(buf: &[u8]) -> Option<String> {
    let tail = buf.strip_prefix(PUNCH_MAGIC.as_slice())?;
    std::str::from_utf8(tail).ok().map(str::to_string)
}

/// Send [`PUNCH_COUNT`] datagrams to every target, [`PUNCH_GAP`] apart.
///
/// Blocking (it sleeps between rounds — about 200 ms total), so call it from a
/// worker thread, never the game loop. Send errors are logged and skipped: an
/// unreachable candidate is exactly the case this exists to work around.
pub fn send_punches(sock: &UdpSocket, targets: &[SocketAddr], session: &str) {
    let d = punch_datagram(session);
    for round in 0..PUNCH_COUNT {
        for t in targets {
            if let Err(e) = sock.send_to(&d, t) {
                log::debug!("[nat] punch {round} to {t}: {e}");
            }
        }
        if round + 1 < PUNCH_COUNT {
            std::thread::sleep(PUNCH_GAP);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RaceOutcome {
    /// This candidate index completed the handshake first.
    Won(usize),
    /// Every candidate was tried and every one failed.
    AllFailed,
    /// The 8 s budget ran out with dials still outstanding.
    TimedOut,
}

/// The parallel-connect decision, as pure state.
pub struct ConnectRace {
    n: usize,
    dialed: usize,
    failed: usize,
    in_flight: Vec<bool>,
    outcome: Option<RaceOutcome>,
}

impl ConnectRace {
    pub fn new(n: usize) -> Self {
        ConnectRace {
            n,
            dialed: 0,
            failed: 0,
            in_flight: vec![false; n],
            // Nothing to try is a loss, immediately — not a wait for the
            // deadline. The joiner gets the right message eight seconds sooner.
            outcome: (n == 0).then_some(RaceOutcome::AllFailed),
        }
    }

    /// Advance the clock to `elapsed` since the race started. Returns every
    /// candidate index whose stagger slot has arrived and which must be dialled
    /// now — a list, not one index, so a caller that is a tick late catches up
    /// rather than skipping candidates.
    pub fn advance(&mut self, elapsed: Duration) -> Vec<usize> {
        if self.outcome.is_some() {
            return Vec::new();
        }
        if elapsed >= CONNECT_DEADLINE {
            self.outcome = Some(RaceOutcome::TimedOut);
            return Vec::new();
        }
        let due = (elapsed.as_millis() / CONNECT_STAGGER.as_millis()) as usize + 1;
        let due = due.min(self.n);
        let mut start = Vec::new();
        while self.dialed < due {
            self.in_flight[self.dialed] = true;
            start.push(self.dialed);
            self.dialed += 1;
        }
        start
    }

    /// Candidate `idx` completed its handshake. Returns the indices to abort:
    /// every OTHER dial still in flight. A second success after a winner is
    /// ignored and returns an empty list — there is exactly one connection.
    pub fn on_connected(&mut self, idx: usize) -> Vec<usize> {
        if self.outcome.is_some() {
            return Vec::new();
        }
        self.outcome = Some(RaceOutcome::Won(idx));
        let abort: Vec<usize> = (0..self.n)
            .filter(|i| *i != idx && self.in_flight[*i])
            .collect();
        for i in &abort {
            self.in_flight[*i] = false;
        }
        self.in_flight[idx] = false;
        abort
    }

    /// Candidate `idx` failed. Only ends the race once every candidate has been
    /// dialled and every one has failed — and never overrides a win.
    pub fn on_failed(&mut self, idx: usize) {
        if self.outcome.is_some() || !self.in_flight[idx] {
            return;
        }
        self.in_flight[idx] = false;
        self.failed += 1;
        if self.failed == self.n {
            self.outcome = Some(RaceOutcome::AllFailed);
        }
    }

    /// `None` while the race is still running.
    pub fn outcome(&self) -> Option<RaceOutcome> {
        self.outcome
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo test --bin axenstax-engine nat::punch 2>&1 | tail -20
```
Expected: PASS — 12 passed.

- [ ] **Step 5: Commit**

```bash
git add game/engine/src/nat/punch.rs && git commit -m "$(cat <<'EOF'
feat(online): punch datagrams + the pure connect-race state machine (P3)

3 punches per candidate 100ms apart, then dials staggered 150ms in priority
order with an 8s deadline. ConnectRace is pure — no sockets, no clock — so
"first success wins, the rest are aborted, there is never a second winner, a
late failure can't undo a win" is a unit test rather than a hope. A race with
no candidates fails at once instead of waiting out the deadline.

Claude-Session: https://claude.ai/code/session_011gW6q5QFBTbF1BNMHiayce
EOF
)" -- game/engine/src/nat/punch.rs
```

---

### Task 14: Socket handoff into quinn (`network.rs`, `hosted_server.rs`, `remote_client.rs`)

**Files:**
- Modify: `game/engine/src/network.rs`
- Modify: `game/engine/src/hosted_server.rs:298-520` (`start`) and `:2317-2377` (`spawn_quic_accept_thread`)
- Modify: `game/engine/src/remote_client.rs`

**Interfaces:**
- Consumes:
  ```rust
  // quinn 0.11
  impl quinn::Endpoint {
      pub fn new(config: EndpointConfig, server_config: Option<ServerConfig>,
                 socket: std::net::UdpSocket, runtime: std::sync::Arc<dyn Runtime>) -> std::io::Result<Self>;
  }
  pub fn quinn::default_runtime() -> Option<std::sync::Arc<dyn Runtime>>;
  // existing engine code
  pub fn network::create_server_endpoint(bind_addr: SocketAddr) -> Result<quinn::Endpoint, Box<dyn std::error::Error>>;
  pub fn network::create_client_endpoint() -> Result<quinn::Endpoint, Box<dyn std::error::Error>>;
  pub fn network::bridge_server_connection(connection: quinn::Connection) -> QuicServerTransport;
  pub fn HostedServer::start(num_local_players: usize, server_name: String, seed: u32,
                             max_remote_players: usize, remote_transport: RemoteTransport) -> Result<Self, String>;
  pub fn RemoteClient::connect_authed(server_addr: SocketAddr, player_name: &str,
                                      driver: SignDriverFn, pinned_op_npub: Option<String>) -> Result<Self, String>;
  pub type SignDriverFn = Box<dyn FnOnce(String, String) -> Receiver<Result<SignedJoin, String>> + Send>;
  ```
- Produces:
  ```rust
  // network.rs
  pub fn create_server_endpoint_on_socket(socket: std::net::UdpSocket)
      -> Result<quinn::Endpoint, Box<dyn std::error::Error>>;
  pub fn create_client_endpoint_on_socket(socket: std::net::UdpSocket)
      -> Result<quinn::Endpoint, Box<dyn std::error::Error>>;
  pub struct OnlineConnect {
      pub transport: QuicClientTransport,
      pub outcome: std::sync::mpsc::Receiver<Result<std::net::SocketAddr, String>>,
  }
  pub fn connect_to_server_on_socket(socket: std::net::UdpSocket,
                                     candidates: Vec<std::net::SocketAddr>,
                                     session: String) -> OnlineConnect;
  // hosted_server.rs
  impl HostedServer {
      pub fn start_online(num_local_players: usize, server_name: String, seed: u32,
                          max_remote_players: usize, socket: std::net::UdpSocket) -> Result<Self, String>;
  }
  // remote_client.rs
  impl RemoteClient {
      pub fn connect_authed_on_transport(transport: Box<dyn ClientTransport>, player_name: &str,
                                         driver: SignDriverFn, pinned_op_npub: Option<String>) -> Self;
  }
  ```

- [ ] **Step 1: Write the failing test**

Append to `game/engine/src/test_integration/handshake.rs`:

```rust
/// A hosted server started on a socket we bound ourselves must listen on THAT
/// port — the whole point of the handoff is that the port a peer was told to
/// dial (in the candidates) is the port QUIC answers on.
#[test]
fn hosted_server_started_on_a_prebound_socket_keeps_its_port() {
    let sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let port = sock.local_addr().unwrap().port();
    let server = crate::hosted_server::HostedServer::start_online(
        1,
        "prebound-test".to_string(),
        42,
        4,
        sock,
    )
    .expect("start_online");
    assert_eq!(server.port, port, "the accept thread must own the socket we bound");
    assert_ne!(port, crate::protocol::SERVER_PORT, "an ephemeral port, not the default");
}

/// A connect race with nothing to dial resolves immediately as an error rather
/// than hanging for the full deadline.
#[test]
fn connect_to_server_on_socket_with_no_candidates_fails_fast() {
    let sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let conn = crate::network::connect_to_server_on_socket(sock, vec![], "sess".to_string());
    let got = conn
        .outcome
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("the race must report an outcome without waiting out the 8s deadline");
    assert!(got.is_err(), "no candidates cannot succeed: {got:?}");
}

/// A candidate nothing is listening on loses, and the race says so before the
/// deadline rather than after it.
#[test]
fn connect_to_server_on_socket_reports_an_unreachable_candidate() {
    let sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    // Bind and immediately drop, so the port is (almost certainly) dead.
    let dead = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let dead_addr = dead.local_addr().unwrap();
    drop(dead);
    let conn =
        crate::network::connect_to_server_on_socket(sock, vec![dead_addr], "sess".to_string());
    let got = conn
        .outcome
        .recv_timeout(crate::nat::punch::CONNECT_DEADLINE + std::time::Duration::from_secs(2))
        .expect("the race must report an outcome");
    assert!(got.is_err(), "a dead candidate cannot win: {got:?}");
}
```

- [ ] **Step 2: Run them to verify they fail**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo test --bin axenstax-engine prebound 2>&1 | tail -20
```
Expected: FAIL — `no function or associated item named start_online`.

- [ ] **Step 3: Extract the shared quinn config and add the socket constructors**

In `game/engine/src/network.rs`, replace `create_server_endpoint` and
`create_client_endpoint` with these five items (the two originals keep their
signatures and behaviour; the configs and the bridge loop are lifted out so the
new constructors are not a second copy):

```rust
/// The server-side TLS + QUIC config. Lifted out of `create_server_endpoint` so
/// the pre-bound-socket variant is the same configuration, not a second copy of
/// it.
fn server_quinn_config() -> Result<quinn::ServerConfig, Box<dyn std::error::Error>> {
    let (cert_der, key_der) = generate_self_signed_cert();
    let cert = rustls::pki_types::CertificateDer::from(cert_der);
    let key = rustls::pki_types::PrivateKeyDer::try_from(key_der)
        .map_err(|e| format!("invalid key: {e}"))?;
    let mut server_crypto = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)?;
    server_crypto.alpn_protocols = vec![ALPN_PROTOCOL.to_vec()];
    Ok(quinn::ServerConfig::with_crypto(std::sync::Arc::new(
        quinn::crypto::rustls::QuicServerConfig::try_from(server_crypto)?,
    )))
}

/// The client-side QUIC config. Certificate verification stays skipped: hosts
/// are self-signed, and the trust anchor is the persona-signed kind-21236 join
/// event (plus the server-identity proof where the host has one), not the TLS
/// certificate.
fn client_quinn_config() -> Result<quinn::ClientConfig, Box<dyn std::error::Error>> {
    let mut client_crypto = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(SkipServerVerification))
        .with_no_client_auth();
    client_crypto.alpn_protocols = vec![ALPN_PROTOCOL.to_vec()];
    Ok(quinn::ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(client_crypto)?,
    )))
}

/// Create a QUIC server endpoint bound to the given address.
pub fn create_server_endpoint(
    bind_addr: SocketAddr,
) -> Result<quinn::Endpoint, Box<dyn std::error::Error>> {
    let endpoint = quinn::Endpoint::server(server_quinn_config()?, bind_addr)?;
    log::info!("QUIC server listening on {bind_addr}");
    Ok(endpoint)
}

/// Create a QUIC server endpoint on an **already-bound** socket.
///
/// This is the online-play path: the socket was bound first, then used to
/// gather candidates (including its STUN reflexive mapping) and to punch
/// through the router, and only then handed here — so the address the peer was
/// told to dial is the address QUIC answers on. Must be called inside a tokio
/// runtime (`default_runtime` needs one).
pub fn create_server_endpoint_on_socket(
    socket: std::net::UdpSocket,
) -> Result<quinn::Endpoint, Box<dyn std::error::Error>> {
    let local = socket.local_addr()?;
    let runtime = quinn::default_runtime()
        .ok_or("no async runtime for quinn — call this inside a tokio context")?;
    let endpoint = quinn::Endpoint::new(
        quinn::EndpointConfig::default(),
        Some(server_quinn_config()?),
        socket,
        runtime,
    )?;
    log::info!("QUIC server listening on the pre-bound socket at {local}");
    Ok(endpoint)
}

/// Create a QUIC client endpoint (binds to 0.0.0.0:0).
pub fn create_client_endpoint() -> Result<quinn::Endpoint, Box<dyn std::error::Error>> {
    let mut endpoint = quinn::Endpoint::client("0.0.0.0:0".parse()?)?;
    endpoint.set_default_client_config(client_quinn_config()?);
    Ok(endpoint)
}

/// Create a QUIC client endpoint on an already-bound socket — the joiner half
/// of the handoff described on [`create_server_endpoint_on_socket`].
pub fn create_client_endpoint_on_socket(
    socket: std::net::UdpSocket,
) -> Result<quinn::Endpoint, Box<dyn std::error::Error>> {
    let runtime = quinn::default_runtime()
        .ok_or("no async runtime for quinn — call this inside a tokio context")?;
    let mut endpoint = quinn::Endpoint::new(
        quinn::EndpointConfig::default(),
        None,
        socket,
        runtime,
    )?;
    endpoint.set_default_client_config(client_quinn_config()?);
    Ok(endpoint)
}
```

- [ ] **Step 4: Extract the bridge loop and add the online connect race**

Still in `network.rs`, add (and rewrite the body of `connect_to_server`'s
`loop { tokio::select! { … } }` to call `bridge_loop`, so there is one copy):

```rust
/// Pump datagrams between a live QUIC connection and the game thread's
/// channels until either side goes away. Shared by every transport bridge so
/// the read/write loop exists once.
async fn bridge_loop(
    conn: quinn::Connection,
    net_tx: mpsc::Sender<Packet>,
    net_rx: mpsc::Receiver<Packet>,
) {
    loop {
        tokio::select! {
            result = conn.read_datagram() => {
                match result {
                    Ok(data) => { let _ = net_tx.send(data.to_vec()); }
                    Err(e) => { log::warn!("QUIC read error: {e}"); break; }
                }
            }
            _ = tokio::time::sleep(tokio::time::Duration::from_millis(1)) => {
                while let Ok(data) = net_rx.try_recv() {
                    if let Err(e) = conn.send_datagram(data.into()) {
                        log::warn!("QUIC send error: {e}");
                        break;
                    }
                }
            }
        }
    }
    log::info!("QUIC connection bridge closed");
}

/// A client transport plus a one-shot report of how the connect race went.
///
/// The transport is returned immediately (like `connect_to_server`) because the
/// game loop wants a handle straight away; `outcome` is how the caller learns
/// whether any candidate actually won, which is what decides between "you're
/// in" and the "couldn't reach" copy.
pub struct OnlineConnect {
    pub transport: QuicClientTransport,
    pub outcome: mpsc::Receiver<Result<SocketAddr, String>>,
}

/// Punch, then race a QUIC connect across every candidate on the pre-bound
/// socket, best candidate first.
///
/// The whole race runs on the worker thread: punching sleeps, and connecting
/// blocks on a handshake, neither of which may happen on the game loop.
pub fn connect_to_server_on_socket(
    socket: std::net::UdpSocket,
    candidates: Vec<SocketAddr>,
    session: String,
) -> OnlineConnect {
    let (game_tx, net_rx) = mpsc::channel::<Packet>();
    let (net_tx, game_rx) = mpsc::channel::<Packet>();
    let (outcome_tx, outcome_rx) = mpsc::channel::<Result<SocketAddr, String>>();

    let handle = std::thread::Builder::new()
        .name("quic-online-connect".into())
        .spawn(move || {
            // Punch BEFORE quinn owns the socket: these datagrams teach the
            // routers that this conversation is wanted, and quinn would drop
            // the replies as not-QUIC anyway.
            crate::nat::punch::send_punches(&socket, &candidates, &session);

            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = outcome_tx.send(Err(format!("no runtime: {e}")));
                    return;
                }
            };
            rt.block_on(async move {
                let endpoint = match create_client_endpoint_on_socket(socket) {
                    Ok(ep) => ep,
                    Err(e) => {
                        let _ = outcome_tx.send(Err(format!("endpoint: {e}")));
                        return;
                    }
                };

                let mut race = crate::nat::punch::ConnectRace::new(candidates.len());
                let started = tokio::time::Instant::now();
                let mut attempts = tokio::task::JoinSet::new();
                let mut winner: Option<(SocketAddr, quinn::Connection)> = None;

                while race.outcome().is_none() {
                    for idx in race.advance(started.elapsed()) {
                        let addr = candidates[idx];
                        match endpoint.connect(addr, "axenstax-server") {
                            Ok(connecting) => {
                                attempts.spawn(async move { (idx, addr, connecting.await) });
                            }
                            Err(e) => {
                                log::debug!("[online] candidate {addr}: {e}");
                                race.on_failed(idx);
                            }
                        }
                    }
                    tokio::select! {
                        Some(joined) = attempts.join_next(), if !attempts.is_empty() => {
                            if let Ok((idx, addr, result)) = joined {
                                match result {
                                    Ok(conn) => {
                                        for _ in race.on_connected(idx) {
                                            // Aborting is implicit: every other
                                            // attempt's Connecting future is
                                            // dropped with the JoinSet below.
                                        }
                                        winner = Some((addr, conn));
                                    }
                                    Err(e) => {
                                        log::debug!("[online] candidate {addr}: {e}");
                                        race.on_failed(idx);
                                    }
                                }
                            }
                        }
                        _ = tokio::time::sleep(tokio::time::Duration::from_millis(20)) => {}
                    }
                }
                attempts.shutdown().await;

                match (race.outcome(), winner) {
                    (Some(crate::nat::punch::RaceOutcome::Won(_)), Some((addr, conn))) => {
                        log::info!("[online] connected to {addr}");
                        let _ = outcome_tx.send(Ok(addr));
                        bridge_loop(conn, net_tx, net_rx).await;
                    }
                    (Some(crate::nat::punch::RaceOutcome::TimedOut), _) => {
                        let _ = outcome_tx.send(Err("timed out".to_string()));
                    }
                    _ => {
                        let _ = outcome_tx.send(Err("no candidate answered".to_string()));
                    }
                }
            });
        })
        .expect("spawn online connect thread");

    OnlineConnect {
        transport: QuicClientTransport {
            tx: game_tx,
            rx: game_rx,
            _network_thread: Some(handle),
        },
        outcome: outcome_rx,
    }
}
```

- [ ] **Step 5: Add `start_online` to `HostedServer`**

In `game/engine/src/hosted_server.rs`:

(a) rename `pub fn start(` to `fn start_inner(` and give it two extra
parameters, keeping the body otherwise unchanged:

```rust
    fn start_inner(
        num_local_players: usize,
        server_name: String,
        seed: u32,
        max_remote_players: usize,
        remote_transport: RemoteTransport,
        #[cfg(not(target_arch = "wasm32"))] prebound: Option<std::net::UdpSocket>,
    ) -> Result<Self, String> {
```

Inside it, replace the `let port = match remote_transport { … };` block with:

```rust
        // Effective listening port. An online host bound its own socket before
        // gathering candidates, so the port is whatever the OS gave it — and it
        // MUST be that one, because that is the port already inside the
        // candidates the peer was sent.
        #[cfg(not(target_arch = "wasm32"))]
        let port = match (&prebound, remote_transport) {
            (Some(s), _) => s
                .local_addr()
                .map_err(|e| format!("pre-bound socket has no address: {e}"))?
                .port(),
            (None, RemoteTransport::Quic) => protocol::SERVER_PORT,
            (None, RemoteTransport::WebSocket { port }) => port,
        };
        #[cfg(target_arch = "wasm32")]
        let port = match remote_transport {
            RemoteTransport::Quic => protocol::SERVER_PORT,
            RemoteTransport::WebSocket { port } => port,
        };
```

and pass `prebound` through to the accept thread:

```rust
                    RemoteTransport::Quic => spawn_quic_accept_thread(
                        port,
                        max_remote_players,
                        current_remote.clone(),
                        shutdown.clone(),
                        tx,
                        prebound,
                    )?,
```

(b) add the two public entry points immediately after `start_inner`:

```rust
    /// Start a hosted server (LAN / dedicated). Unchanged behaviour: the accept
    /// thread binds its own socket.
    pub fn start(
        num_local_players: usize,
        server_name: String,
        seed: u32,
        max_remote_players: usize,
        remote_transport: RemoteTransport,
    ) -> Result<Self, String> {
        Self::start_inner(
            num_local_players,
            server_name,
            seed,
            max_remote_players,
            remote_transport,
            #[cfg(not(target_arch = "wasm32"))]
            None,
        )
    }

    /// Start a hosted server on an **already-bound** UDP socket (online play by
    /// contact, spec §4.2).
    ///
    /// The caller bound the socket, gathered candidates on it (so its STUN
    /// mapping is the one QUIC will use), punched with a clone of it, and now
    /// hands the original over. `require_signin` stays `true` — this is the
    /// QUIC path — and the caller feeds the contacts-plus-bearer allowlist in
    /// through `set_access_policy`.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn start_online(
        num_local_players: usize,
        server_name: String,
        seed: u32,
        max_remote_players: usize,
        socket: std::net::UdpSocket,
    ) -> Result<Self, String> {
        Self::start_inner(
            num_local_players,
            server_name,
            seed,
            max_remote_players,
            RemoteTransport::Quic,
            Some(socket),
        )
    }
```

(c) change `spawn_quic_accept_thread` to take and use the socket:

```rust
fn spawn_quic_accept_thread(
    port: u16,
    max_remote_players: usize,
    current_remote: Arc<AtomicUsize>,
    shutdown: Arc<AtomicBool>,
    remote_tx: mpsc::Sender<Box<dyn ServerTransport>>,
    prebound: Option<std::net::UdpSocket>,
) -> Result<thread::JoinHandle<()>, String> {
```

and inside its `rt.block_on(async move { … })`, replace the endpoint creation with:

```rust
                let endpoint = match prebound {
                    // Online play: quinn adopts the socket we already bound,
                    // gathered candidates on, and punched from.
                    Some(sock) => match crate::network::create_server_endpoint_on_socket(sock) {
                        Ok(ep) => ep,
                        Err(e) => {
                            log::error!("Failed to adopt the pre-bound QUIC socket: {e}");
                            return;
                        }
                    },
                    None => {
                        let bind_addr: SocketAddr = format!("0.0.0.0:{port}")
                            .parse()
                            .expect("valid bind address");
                        match crate::network::create_server_endpoint(bind_addr) {
                            Ok(ep) => {
                                log::info!("QUIC server listening on port {port}");
                                ep
                            }
                            Err(e) => {
                                log::error!("Failed to create QUIC endpoint: {e}");
                                return;
                            }
                        }
                    }
                };
```

(d) make `port` readable by the test — it is already `pub port: u16` with an
`#[allow(dead_code)]`; change the attribute's comment to note the new reader:

```rust
    /// The port the accept thread is listening on. For an online host this is
    /// the pre-bound socket's port, which is also what the peer was told to
    /// dial. Read by `online_host` for the Online panel and by the
    /// pre-bound-socket integration test.
    pub port: u16,
```
(and delete the `#[allow(dead_code)]` above it — it now has readers).

- [ ] **Step 6: Add `connect_authed_on_transport` to `RemoteClient`**

In `game/engine/src/remote_client.rs`, immediately after `connect_authed`:

```rust
    /// Authenticated join over a transport the caller already built.
    ///
    /// The online path needs this because the connection is not made by dialling
    /// one address: it is the winner of a race across several candidates on a
    /// pre-bound socket (`network::connect_to_server_on_socket`). Everything
    /// after that — challenge, sign, JoinRequest — is the SAME handshake
    /// `connect_authed` runs, unchanged.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn connect_authed_on_transport(
        transport: Box<dyn ClientTransport>,
        player_name: &str,
        driver: SignDriverFn,
        pinned_op_npub: Option<String>,
    ) -> Self {
        log::info!("Joining over an online transport (QUIC, authed)...");
        Self::from_transport_authed(
            transport,
            build_join_request_guest(player_name, 0),
            driver,
            pinned_op_npub,
        )
    }
```

- [ ] **Step 7: Run the tests to verify they pass**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo test --bin axenstax-engine 2>&1 | tail -20
```
Expected: PASS — the whole suite, including the three new handshake tests. The
existing LAN-host tests still pass because `start` is behaviour-identical.

- [ ] **Step 8: Verify clippy and the wasm build**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo clippy -- -D warnings 2>&1 | tail -20
```
Expected: no warnings. Then:
```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 trunk build 2>&1 | tail -10
```
Expected: `success`. `start_inner`'s `prebound` parameter is `#[cfg]`-gated, so
the wasm build never sees a `std::net::UdpSocket`.

- [ ] **Step 9: Commit**

```bash
git add game/engine/src/network.rs game/engine/src/hosted_server.rs game/engine/src/remote_client.rs game/engine/src/test_integration/handshake.rs && git commit -m "$(cat <<'EOF'
feat(online): hand a pre-bound UDP socket to quinn, both sides (P3)

create_{server,client}_endpoint_on_socket adopt an already-bound socket, so the
address a peer was told to dial (STUN mapping included) is the address QUIC
answers on. HostedServer::start delegates to start_inner and start_online is the
online entry point; the accept thread adopts the socket instead of binding.
connect_to_server_on_socket punches, then races the candidates and reports the
outcome on a channel. The TLS/QUIC configs and the datagram bridge loop are
extracted so none of this is a second copy.

Claude-Session: https://claude.ai/code/session_011gW6q5QFBTbF1BNMHiayce
EOF
)" -- game/engine/src/network.rs game/engine/src/hosted_server.rs game/engine/src/remote_client.rs game/engine/src/test_integration/handshake.rs
```

---

## Phase 4 — Orchestration, UI, docs

### Task 15: Host-side orchestration (`online_host.rs`)

**Files:**
- Create: `game/engine/src/online_host.rs`
- Modify: `game/engine/src/main.rs`

**Interfaces:**
- Consumes: `invite::{Invite, mint_bearer, DEFAULT_INVITE_TTL_SECS}`,
  `runtime_identity::RuntimeIdentity`, `contacts::{Contact, AddedVia, upsert, save_mirror, mirror_path}`,
  `online_admission::{admit, is_reply_worthy, refusal_wire, ActiveBearer, Admission, AdmitReason, Refusal}`,
  `rendezvous::payload::{Answer, Candidate, KIND_JOIN_OFFER, PAYLOAD_VERSION, npub_of, seal_answer}`,
  `rendezvous::verify::{verify_offer, SessionGuard}`,
  `rendezvous::relay_client::RendezvousRelay`,
  `nat::{candidates::parse_addrs, punch::send_punches}`.
- Produces:
  ```rust
  pub fn unreachable_warning() -> &'static str;
  #[derive(Clone, Debug, PartialEq, Eq)]
  pub enum HostEvent {
      Admitted { persona: [u8; 32], reason: AdmitReason },
      AllowlistChanged(Vec<[u8; 32]>),
      ContactAdded(Box<Contact>),
      Dropped(String),
  }
  pub struct OnlineHost { /* private */ }
  impl OnlineHost {
      pub fn start(identity: RuntimeIdentity, persona: [u8; 32],
                   relay: Box<dyn RendezvousRelay>, punch_socket: std::net::UdpSocket,
                   candidates: Vec<Candidate>, world_name: String,
                   relays: Vec<String>, book: Vec<Contact>, now: u64) -> Result<OnlineHost, String>;
      pub fn invite(&self) -> &Invite;
      pub fn invite_link(&self) -> String;
      pub fn candidates(&self) -> &[Candidate];
      pub fn relays_connected(&self) -> (usize, usize);
      pub fn allowlist(&self) -> &[[u8; 32]];
      pub fn poll(&mut self, now: u64, players: usize, capacity: usize) -> Vec<HostEvent>;
      pub fn mint_fresh_invite(&mut self, now: u64) -> &Invite;
  }
  ```

- [ ] **Step 1: Write the failing tests**

Create `game/engine/src/online_host.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendezvous::payload::{new_session_id, seal_offer, Offer};
    use crate::rendezvous::relay_client::{FakeRelayHub, RendezvousRelay};
    use nostr::Keys;

    const NOW: u64 = 1_700_000_000;

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    struct Caller {
        persona: Keys,
        runtime: Keys,
    }

    impl Caller {
        fn new() -> Self {
            Caller { persona: Keys::generate(), runtime: Keys::generate() }
        }
        async fn offer(&self, bearer: Option<String>, protocol: u32) -> Offer {
            Offer {
                v: PAYLOAD_VERSION,
                session: new_session_id(),
                persona: npub_of(&self.persona.public_key()),
                attestation: crate::runtime_identity::mint_player_attestation(
                    &self.persona,
                    &self.runtime.public_key(),
                    nostr::Timestamp::from(NOW - 10),
                    90,
                )
                .await
                .unwrap(),
                bearer,
                protocol,
                candidates: vec![Candidate {
                    kind: "lan".to_string(),
                    addr: "127.0.0.1:9".to_string(),
                }],
                sent_at: NOW,
            }
        }
    }

    /// A host wired to a fake relay, with `book` as its contacts.
    fn a_host(hub: &FakeRelayHub, book: Vec<Contact>) -> (OnlineHost, Keys) {
        let host_persona = Keys::generate();
        let host_runtime = Keys::generate();
        let attestation = rt().block_on(crate::runtime_identity::mint_player_attestation(
            &host_persona,
            &host_runtime.public_key(),
            nostr::Timestamp::from(NOW - 10),
            90,
        ))
        .unwrap();
        let identity = RuntimeIdentity::from_parts(host_runtime.clone(), Some(attestation));
        let sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let host = OnlineHost::start(
            identity,
            host_persona.public_key().to_bytes(),
            Box::new(hub.client()),
            sock,
            vec![Candidate { kind: "lan".to_string(), addr: "127.0.0.1:7700".to_string() }],
            "Ivy's Hollow".to_string(),
            vec!["wss://nos.lol".to_string()],
            book,
            NOW,
        )
        .unwrap();
        (host, host_runtime)
    }

    fn a_contact(pk: [u8; 32], tier: crate::comms::Tier) -> Contact {
        Contact {
            pubkey: pk,
            display_name: Some("Friend".to_string()),
            tier,
            is_child: false,
            runtime_pubkey: None,
            added_via: AddedVia::Kenspeckle,
            added_at: 0,
        }
    }

    #[test]
    fn the_minted_invite_round_trips_and_names_this_host() {
        let hub = FakeRelayHub::new();
        let (host, host_runtime) = a_host(&hub, vec![]);
        let parsed = crate::invite::Invite::parse(&host.invite_link(), NOW).unwrap();
        assert_eq!(parsed.host_runtime, host_runtime.public_key().to_bytes());
        assert_eq!(parsed.world_name, "Ivy's Hollow");
        assert_eq!(parsed.relays, vec!["wss://nos.lol".to_string()]);
        assert_eq!(parsed.expires_at, NOW + crate::invite::DEFAULT_INVITE_TTL_SECS);
    }

    #[test]
    fn minting_a_fresh_invite_retires_the_old_bearer() {
        let hub = FakeRelayHub::new();
        let (mut host, _) = a_host(&hub, vec![]);
        let old = host.invite().bearer;
        let new = host.mint_fresh_invite(NOW + 60).bearer;
        assert_ne!(old, new);
    }

    #[test]
    fn a_kith_contact_is_admitted_and_lands_on_the_allowlist() {
        rt().block_on(async {
            let hub = FakeRelayHub::new();
            let caller = Caller::new();
            let (mut host, host_runtime) = a_host(
                &hub,
                vec![a_contact(caller.persona.public_key().to_bytes(), crate::comms::Tier::Kith)],
            );
            let joiner_relay = hub.client();
            let offer = caller.offer(None, crate::protocol::PROTOCOL_VERSION).await;
            let ev = seal_offer(&caller.runtime, &host_runtime.public_key(), &offer)
                .await
                .unwrap();
            joiner_relay.publish(&ev).unwrap();

            let events = host.poll(NOW, 1, 5);
            assert!(events.iter().any(|e| matches!(
                e,
                HostEvent::Admitted { reason: AdmitReason::AlreadyContact, .. }
            )), "{events:?}");
            assert!(host.allowlist().contains(&caller.persona.public_key().to_bytes()));
        });
    }

    #[test]
    fn a_stranger_with_the_bearer_is_admitted_and_becomes_a_kith_contact() {
        rt().block_on(async {
            let hub = FakeRelayHub::new();
            let caller = Caller::new();
            let (mut host, host_runtime) = a_host(&hub, vec![]);
            let bearer = hex::encode(host.invite().bearer);
            let joiner_relay = hub.client();
            let offer = caller.offer(Some(bearer), crate::protocol::PROTOCOL_VERSION).await;
            let ev = seal_offer(&caller.runtime, &host_runtime.public_key(), &offer)
                .await
                .unwrap();
            joiner_relay.publish(&ev).unwrap();

            let events = host.poll(NOW, 1, 5);
            let added = events.iter().find_map(|e| match e {
                HostEvent::ContactAdded(c) => Some(c.clone()),
                _ => None,
            });
            let added = added.expect("an invite admits AND makes a contact");
            assert_eq!(added.pubkey, caller.persona.public_key().to_bytes());
            assert_eq!(added.tier, crate::comms::Tier::Kith);
            assert_eq!(added.added_via, AddedVia::Invite);
            assert_eq!(
                added.runtime_pubkey,
                Some(caller.runtime.public_key().to_bytes()),
                "the rendezvous is where a runtime key can be learned"
            );
        });
    }

    #[test]
    fn a_stranger_with_no_bearer_gets_no_answer_at_all() {
        rt().block_on(async {
            let hub = FakeRelayHub::new();
            let caller = Caller::new();
            let (mut host, host_runtime) = a_host(&hub, vec![]);
            let joiner_relay = hub.client();
            joiner_relay
                .subscribe(KIND_JOIN_ANSWER, &caller.runtime.public_key().to_hex())
                .unwrap();
            let offer = caller.offer(None, crate::protocol::PROTOCOL_VERSION).await;
            let ev = seal_offer(&caller.runtime, &host_runtime.public_key(), &offer)
                .await
                .unwrap();
            joiner_relay.publish(&ev).unwrap();

            let events = host.poll(NOW, 1, 5);
            assert!(events.iter().any(|e| matches!(e, HostEvent::Dropped(_))), "{events:?}");
            assert!(joiner_relay.try_recv().is_none(), "SILENCE — no answer to a stranger");
            assert!(host.allowlist().is_empty());
        });
    }

    #[test]
    fn a_ken_contact_is_refused_like_a_stranger() {
        rt().block_on(async {
            let hub = FakeRelayHub::new();
            let caller = Caller::new();
            let (mut host, host_runtime) = a_host(
                &hub,
                vec![a_contact(caller.persona.public_key().to_bytes(), crate::comms::Tier::Ken)],
            );
            let joiner_relay = hub.client();
            joiner_relay
                .subscribe(KIND_JOIN_ANSWER, &caller.runtime.public_key().to_hex())
                .unwrap();
            let offer = caller.offer(None, crate::protocol::PROTOCOL_VERSION).await;
            let ev = seal_offer(&caller.runtime, &host_runtime.public_key(), &offer)
                .await
                .unwrap();
            joiner_relay.publish(&ev).unwrap();
            host.poll(NOW, 1, 5);
            assert!(joiner_relay.try_recv().is_none(), "ken is hear-only, not play-with");
            assert!(host.allowlist().is_empty());
        });
    }

    #[test]
    fn a_protocol_mismatch_is_explained_rather_than_ignored() {
        rt().block_on(async {
            let hub = FakeRelayHub::new();
            let caller = Caller::new();
            let (mut host, host_runtime) = a_host(
                &hub,
                vec![a_contact(caller.persona.public_key().to_bytes(), crate::comms::Tier::Kin)],
            );
            let joiner_relay = hub.client();
            joiner_relay
                .subscribe(KIND_JOIN_ANSWER, &caller.runtime.public_key().to_hex())
                .unwrap();
            let offer = caller
                .offer(None, crate::protocol::PROTOCOL_VERSION + 1)
                .await;
            let ev = seal_offer(&caller.runtime, &host_runtime.public_key(), &offer)
                .await
                .unwrap();
            joiner_relay.publish(&ev).unwrap();
            host.poll(NOW, 1, 5);

            let answer_ev = joiner_relay.try_recv().expect("a mismatch IS answered");
            let answer =
                crate::rendezvous::payload::open_answer(&caller.runtime, &answer_ev).unwrap();
            assert!(!answer.accepted);
            assert_eq!(answer.reason.as_deref(), Some("protocol-mismatch"));
            assert!(host.allowlist().is_empty(), "an explained refusal is still a refusal");
        });
    }

    #[test]
    fn a_full_world_is_explained() {
        rt().block_on(async {
            let hub = FakeRelayHub::new();
            let caller = Caller::new();
            let (mut host, host_runtime) = a_host(
                &hub,
                vec![a_contact(caller.persona.public_key().to_bytes(), crate::comms::Tier::Kin)],
            );
            let joiner_relay = hub.client();
            joiner_relay
                .subscribe(KIND_JOIN_ANSWER, &caller.runtime.public_key().to_hex())
                .unwrap();
            let offer = caller.offer(None, crate::protocol::PROTOCOL_VERSION).await;
            let ev = seal_offer(&caller.runtime, &host_runtime.public_key(), &offer)
                .await
                .unwrap();
            joiner_relay.publish(&ev).unwrap();
            host.poll(NOW, 5, 5); // players == capacity

            let answer_ev = joiner_relay.try_recv().expect("a full world IS answered");
            let answer =
                crate::rendezvous::payload::open_answer(&caller.runtime, &answer_ev).unwrap();
            assert_eq!(answer.reason.as_deref(), Some("full"));
        });
    }

    #[test]
    fn an_accepted_answer_carries_the_hosts_candidates_and_world_name() {
        rt().block_on(async {
            let hub = FakeRelayHub::new();
            let caller = Caller::new();
            let (mut host, host_runtime) = a_host(
                &hub,
                vec![a_contact(caller.persona.public_key().to_bytes(), crate::comms::Tier::Kith)],
            );
            let joiner_relay = hub.client();
            joiner_relay
                .subscribe(KIND_JOIN_ANSWER, &caller.runtime.public_key().to_hex())
                .unwrap();
            let offer = caller.offer(None, crate::protocol::PROTOCOL_VERSION).await;
            let ev = seal_offer(&caller.runtime, &host_runtime.public_key(), &offer)
                .await
                .unwrap();
            joiner_relay.publish(&ev).unwrap();
            host.poll(NOW, 1, 5);

            let answer_ev = joiner_relay.try_recv().unwrap();
            let answer =
                crate::rendezvous::payload::open_answer(&caller.runtime, &answer_ev).unwrap();
            assert!(answer.accepted);
            assert_eq!(answer.session, offer.session, "the answer is tied to the offer");
            assert_eq!(answer.world_name, "Ivy's Hollow");
            assert_eq!(answer.candidates[0].addr, "127.0.0.1:7700");
        });
    }

    #[test]
    fn the_same_offer_twice_is_admitted_once() {
        rt().block_on(async {
            let hub = FakeRelayHub::new();
            let caller = Caller::new();
            let (mut host, host_runtime) = a_host(
                &hub,
                vec![a_contact(caller.persona.public_key().to_bytes(), crate::comms::Tier::Kith)],
            );
            let joiner_relay = hub.client();
            let offer = caller.offer(None, crate::protocol::PROTOCOL_VERSION).await;
            let ev = seal_offer(&caller.runtime, &host_runtime.public_key(), &offer)
                .await
                .unwrap();
            joiner_relay.publish(&ev).unwrap();
            joiner_relay.publish(&ev).unwrap();
            let events = host.poll(NOW, 1, 5);
            let admits = events
                .iter()
                .filter(|e| matches!(e, HostEvent::Admitted { .. }))
                .count();
            assert_eq!(admits, 1, "the replay guard holds: {events:?}");
        });
    }

    #[test]
    fn the_unreachable_warning_is_the_approved_copy() {
        assert_eq!(
            unreachable_warning(),
            "Friends outside your home probably can't reach you. Turn on UPnP on your router."
        );
    }
}
```

- [ ] **Step 2: Run them to verify they fail**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo test --bin axenstax-engine online_host 2>&1 | tail -20
```
Expected: FAIL — `file not found for module online_host`.

- [ ] **Step 3: Add the test constructor `RuntimeIdentity::from_parts`**

In `game/engine/src/runtime_identity.rs`, inside `impl RuntimeIdentity`:

```rust
    /// Build an identity from parts. Production goes through [`load`]; this is
    /// how `online_host`/`online_join` tests stand one up without touching the
    /// filesystem or a bunker.
    pub fn from_parts(keys: Keys, attestation: Option<Event>) -> RuntimeIdentity {
        RuntimeIdentity { keys, attestation }
    }
```

- [ ] **Step 4: Write the implementation**

Put this above the test module in `game/engine/src/online_host.rs`:

```rust
//! Hosting a world for a friend in another house.
//!
//! The host publishes nothing about its world anywhere. It subscribes to
//! kind-20900 events addressed to its own runtime key, and every offer that
//! arrives is either from somebody already in its contacts book or from
//! somebody holding an invite the host minted itself. There is no directory, no
//! listing, and no way to arrive here without having been given something first
//! (CLAUDE.md red line 1).
//!
//! `poll` is called from the game loop and never blocks: relay reads are
//! `try_recv`, and the one blocking thing — punching, which sleeps ~200 ms —
//! goes to a worker thread.
//!
//! Spec: `docs/superpowers/specs/2026-09-06-online-play-by-contact-design.md` §5.1.
#![cfg(not(target_arch = "wasm32"))]

use std::net::UdpSocket;
use std::sync::Arc;

use crate::contacts::{mirror_path, save_mirror, upsert, AddedVia, Contact};
use crate::invite::{mint_bearer, Invite, DEFAULT_INVITE_TTL_SECS};
use crate::nat::candidates::parse_addrs;
use crate::online_admission::{
    admit, is_reply_worthy, refusal_wire, ActiveBearer, AdmitReason, Admission, Refusal,
};
use crate::rendezvous::payload::{
    npub_of, seal_answer, Answer, Candidate, KIND_JOIN_ANSWER, KIND_JOIN_OFFER, PAYLOAD_VERSION,
};
use crate::rendezvous::relay_client::RendezvousRelay;
use crate::rendezvous::verify::{verify_offer, SessionGuard};
use crate::runtime_identity::RuntimeIdentity;

/// Shown when a host has gathered no candidate that could work from outside the
/// house. Hosting still starts — the LAN path is real and useful — but saying
/// nothing would leave the player wondering why their friend never arrives.
pub fn unreachable_warning() -> &'static str {
    "Friends outside your home probably can't reach you. Turn on UPnP on your router."
}

/// Something the game loop needs to act on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostEvent {
    Admitted {
        persona: [u8; 32],
        reason: AdmitReason,
    },
    /// Push this into `HostedServer::set_access_policy`.
    AllowlistChanged(Vec<[u8; 32]>),
    /// A new contact was made; the mirror has already been written.
    ContactAdded(Box<Contact>),
    /// An offer was discarded. Carries a reason for the log only — nothing was
    /// sent back.
    Dropped(String),
}

pub struct OnlineHost {
    identity: RuntimeIdentity,
    persona: [u8; 32],
    relay: Box<dyn RendezvousRelay>,
    /// A `try_clone` of the socket quinn owns, so punches leave from the same
    /// source port the peer was told to expect. Shared with the punch worker.
    punch_socket: Arc<UdpSocket>,
    candidates: Vec<Candidate>,
    world_name: String,
    relays: Vec<String>,
    book: Vec<Contact>,
    allowlist: Vec<[u8; 32]>,
    invite: Invite,
    active_bearer: ActiveBearer,
    guard: SessionGuard,
}

impl OnlineHost {
    /// Mint the first invite and subscribe for offers.
    ///
    /// `punch_socket` should be a `try_clone()` of the socket handed to
    /// `HostedServer::start_online`; `candidates` are what
    /// `nat::candidates::gather` found on it.
    #[allow(clippy::too_many_arguments)] // every one is a distinct dependency
    pub fn start(
        identity: RuntimeIdentity,
        persona: [u8; 32],
        relay: Box<dyn RendezvousRelay>,
        punch_socket: UdpSocket,
        candidates: Vec<Candidate>,
        world_name: String,
        relays: Vec<String>,
        book: Vec<Contact>,
        now: u64,
    ) -> Result<OnlineHost, String> {
        let runtime_pk = identity.runtime_pubkey();
        relay.subscribe(KIND_JOIN_OFFER, &runtime_pk.to_hex())?;

        let bearer = mint_bearer();
        let expires_at = now + DEFAULT_INVITE_TTL_SECS;
        let invite = Invite {
            host_persona: npub_of(&nostr::PublicKey::from_slice(&persona).map_err(|e| e.to_string())?),
            host_runtime: runtime_pk.to_bytes(),
            relays: relays.clone(),
            bearer,
            expires_at,
            world_name: world_name.clone(),
        };

        Ok(OnlineHost {
            identity,
            persona,
            relay,
            punch_socket: Arc::new(punch_socket),
            candidates,
            world_name,
            relays,
            book,
            allowlist: Vec::new(),
            invite,
            active_bearer: ActiveBearer { bearer, expires_at },
            guard: SessionGuard::new(),
        })
    }

    pub fn invite(&self) -> &Invite {
        &self.invite
    }

    pub fn invite_link(&self) -> String {
        self.invite.to_link()
    }

    pub fn candidates(&self) -> &[Candidate] {
        &self.candidates
    }

    pub fn relays_connected(&self) -> (usize, usize) {
        self.relay.connected()
    }

    /// The personas `HostedServer::set_access_policy` should be given, on top of
    /// the host's own contacts. Grows as bearers admit people.
    pub fn allowlist(&self) -> &[[u8; 32]] {
        &self.allowlist
    }

    /// Mint a new invite, retiring the previous bearer.
    pub fn mint_fresh_invite(&mut self, now: u64) -> &Invite {
        let bearer = mint_bearer();
        let expires_at = now + DEFAULT_INVITE_TTL_SECS;
        self.invite = Invite {
            host_persona: self.invite.host_persona.clone(),
            host_runtime: self.invite.host_runtime,
            relays: self.relays.clone(),
            bearer,
            expires_at,
            world_name: self.world_name.clone(),
        };
        self.active_bearer = ActiveBearer { bearer, expires_at };
        &self.invite
    }

    /// Drain and answer everything the relays have delivered. Non-blocking.
    pub fn poll(&mut self, now: u64, players: usize, capacity: usize) -> Vec<HostEvent> {
        let mut out = Vec::new();
        while let Some(ev) = self.relay.try_recv() {
            let verified = match verify_offer(&ev, self.identity.keys(), &mut self.guard, now) {
                Ok(v) => v,
                Err(e) => {
                    // Silence. The variant is for the log, not for the caller.
                    out.push(HostEvent::Dropped(format!("{e:?}")));
                    continue;
                }
            };

            // Capacity and version first: both are refusals we WILL explain, and
            // neither depends on who is asking.
            let decision = if verified.offer.protocol != crate::protocol::PROTOCOL_VERSION {
                Admission::Refuse(Refusal::ProtocolMismatch)
            } else if players >= capacity {
                Admission::Refuse(Refusal::Full)
            } else {
                let bearer = verified
                    .offer
                    .bearer
                    .as_deref()
                    .and_then(|h| hex::decode(h).ok())
                    .and_then(|b| <[u8; 16]>::try_from(b.as_slice()).ok());
                admit(
                    &verified.persona,
                    bearer,
                    &self.book,
                    Some(&self.active_bearer),
                    now,
                )
            };

            match decision {
                Admission::Accept(reason) => {
                    if reason == AdmitReason::ByInvite {
                        let contact = Contact {
                            pubkey: verified.persona,
                            display_name: None,
                            tier: crate::comms::Tier::Kith,
                            is_child: false,
                            runtime_pubkey: Some(verified.runtime),
                            added_via: AddedVia::Invite,
                            added_at: now,
                        };
                        upsert(&mut self.book, contact.clone());
                        if let Err(e) = save_mirror(&mirror_path(), &self.book) {
                            log::warn!("[online] could not write the contacts mirror: {e}");
                        }
                        out.push(HostEvent::ContactAdded(Box::new(contact)));
                    }
                    if !self.allowlist.contains(&verified.persona) {
                        self.allowlist.push(verified.persona);
                        out.push(HostEvent::AllowlistChanged(self.allowlist.clone()));
                    }
                    out.push(HostEvent::Admitted {
                        persona: verified.persona,
                        reason,
                    });

                    // Punch toward the joiner BEFORE answering, so by the time
                    // they start dialling, this router already expects them.
                    self.punch(&verified.offer.candidates, &verified.offer.session);
                    self.send_answer(&verified.runtime, &verified.offer.session, true, None, now);
                }
                Admission::Refuse(r) => {
                    out.push(HostEvent::Dropped(format!("refused: {r:?}")));
                    if is_reply_worthy(r) {
                        self.send_answer(
                            &verified.runtime,
                            &verified.offer.session,
                            false,
                            Some(refusal_wire(r)),
                            now,
                        );
                    }
                }
            }
        }
        out
    }

    /// Fire punches at the joiner's candidates on a worker thread — the send
    /// loop sleeps ~200 ms and must never do that on the game loop.
    fn punch(&self, candidates: &[Candidate], session: &str) {
        let targets = parse_addrs(candidates);
        if targets.is_empty() {
            return;
        }
        let sock = Arc::clone(&self.punch_socket);
        let session = session.to_string();
        std::thread::Builder::new()
            .name("online-punch".into())
            .spawn(move || crate::nat::punch::send_punches(&sock, &targets, &session))
            .map(|_| ())
            .unwrap_or_else(|e| log::warn!("[online] could not spawn the punch thread: {e}"));
    }

    fn send_answer(
        &self,
        to_runtime: &[u8; 32],
        session: &str,
        accepted: bool,
        reason: Option<&str>,
        now: u64,
    ) {
        let Some(attestation) = self.identity.attestation().cloned() else {
            log::warn!("[online] cannot answer without an attestation");
            return;
        };
        let Ok(recipient) = nostr::PublicKey::from_slice(to_runtime) else {
            return;
        };
        let answer = Answer {
            v: PAYLOAD_VERSION,
            session: session.to_string(),
            persona: self.invite.host_persona.clone(),
            attestation,
            accepted,
            reason: reason.map(str::to_string),
            protocol: crate::protocol::PROTOCOL_VERSION,
            // A refusal names no addresses — there is no reason to hand them to
            // somebody who is not coming in.
            candidates: if accepted {
                self.candidates.clone()
            } else {
                Vec::new()
            },
            world_name: self.world_name.clone(),
            sent_at: now,
        };
        let keys = self.identity.keys().clone();
        let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(rt) => rt,
            Err(e) => {
                log::warn!("[online] no runtime to seal an answer: {e}");
                return;
            }
        };
        match rt.block_on(seal_answer(&keys, &recipient, &answer)) {
            Ok(ev) => {
                debug_assert_eq!(ev.kind, nostr::Kind::Custom(KIND_JOIN_ANSWER));
                if let Err(e) = self.relay.publish(&ev) {
                    log::warn!("[online] could not publish the answer: {e}");
                }
            }
            Err(e) => log::warn!("[online] could not seal the answer: {e}"),
        }
    }
}
```

- [ ] **Step 5: Register the module**

In `game/engine/src/main.rs`:

```rust
// Online play by contact — host-side orchestration.
#[cfg(not(target_arch = "wasm32"))]
mod online_host;
```

- [ ] **Step 6: Run the tests to verify they pass**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo test --bin axenstax-engine online_host 2>&1 | tail -20
```
Expected: PASS — 11 passed. The two that matter most are
`a_stranger_with_no_bearer_gets_no_answer_at_all` and
`a_ken_contact_is_refused_like_a_stranger`: both assert the joiner's inbox is
**empty**, which is the silence rule.

- [ ] **Step 7: Commit**

```bash
git add game/engine/src/online_host.rs game/engine/src/runtime_identity.rs game/engine/src/main.rs && git commit -m "$(cat <<'EOF'
feat(online): host-side orchestration — verify, admit, punch, answer (P4)

Subscribes only to offers addressed to its own runtime key; publishes nothing
about the world anywhere. Capacity and version are decided first (they are the
two refusals we explain), then the identity rule. A stranger and a ken contact
both get SILENCE — asserted by checking the joiner's inbox is empty. An invite
admission writes the new Kith contact, with the runtime key learned from the
rendezvous, straight into the mirror.

Claude-Session: https://claude.ai/code/session_011gW6q5QFBTbF1BNMHiayce
EOF
)" -- game/engine/src/online_host.rs game/engine/src/runtime_identity.rs game/engine/src/main.rs
```

---

### Task 16: Joiner-side orchestration + the failure copy (`online_join.rs`)

**Files:**
- Create: `game/engine/src/online_join.rs`
- Modify: `game/engine/src/main.rs`

**Interfaces:**
- Consumes: everything Task 15 consumes, plus
  `rendezvous::verify::verify_answer`, `network::{connect_to_server_on_socket, OnlineConnect}`,
  `online_admission::refusal_from_wire`, `nat::candidates::parse_addrs`.
- Produces:
  ```rust
  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  pub enum JoinFailure { NoAnswer, NoConnect, ProtocolMismatch, Full }
  pub fn failure_copy(failure: JoinFailure, name: &str) -> String;
  pub const ANSWER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8);

  #[derive(Clone, Debug, PartialEq, Eq)]
  pub enum JoinStep {
      Waiting,
      Connecting { world_name: String },
      Ready { world_name: String, host_persona: [u8; 32] },
      Failed(String),
  }
  pub struct OnlineJoin { /* private */ }
  impl OnlineJoin {
      pub fn start(identity: RuntimeIdentity, persona: [u8; 32],
                   relay: Box<dyn RendezvousRelay>, socket: std::net::UdpSocket,
                   candidates: Vec<Candidate>, host_runtime: [u8; 32],
                   bearer: Option<[u8; 16]>, display_name: String, now: u64)
                   -> Result<OnlineJoin, String>;
      pub fn session(&self) -> &str;
      pub fn poll(&mut self, now: u64, elapsed: std::time::Duration) -> JoinStep;
      pub fn take_transport(&mut self) -> Option<Box<dyn crate::transport::ClientTransport>>;
      pub fn host_contact(&self) -> Option<Contact>;
  }
  ```

- [ ] **Step 1: Write the failing tests**

Create `game/engine/src/online_join.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendezvous::payload::{seal_answer, Answer, KIND_JOIN_OFFER, PAYLOAD_VERSION};
    use crate::rendezvous::relay_client::{FakeRelayHub, RendezvousRelay};
    use nostr::Keys;

    const NOW: u64 = 1_700_000_000;

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    // ─── The failure copy, verbatim from spec §4.4 ───

    #[test]
    fn no_answer_copy_is_verbatim() {
        assert_eq!(
            failure_copy(JoinFailure::NoAnswer, "Rowan"),
            "Rowan didn't answer. Are they online with the world open?"
        );
    }

    #[test]
    fn no_connect_copy_is_verbatim() {
        assert_eq!(
            failure_copy(JoinFailure::NoConnect, "Rowan"),
            "Couldn't reach Rowan's world. Their router needs UPnP turned on, or you both \
             need IPv6. Ask them to check Settings → Online in the game."
        );
    }

    #[test]
    fn protocol_mismatch_copy_is_verbatim() {
        assert_eq!(
            failure_copy(JoinFailure::ProtocolMismatch, "Rowan"),
            "You're on different versions. One of you needs to update."
        );
    }

    #[test]
    fn full_copy_is_verbatim() {
        assert_eq!(
            failure_copy(JoinFailure::Full, "Rowan"),
            "Rowan's world is full."
        );
    }

    #[test]
    fn the_failure_copy_is_kid_readable_uk_english_and_names_no_jargon() {
        for f in [
            JoinFailure::NoAnswer,
            JoinFailure::NoConnect,
            JoinFailure::ProtocolMismatch,
            JoinFailure::Full,
        ] {
            let s = failure_copy(f, "Rowan");
            for jargon in ["NAT", "STUN", "QUIC", "npub", "socket", "candidate", "relay"] {
                assert!(!s.contains(jargon), "{f:?} copy says {jargon:?}: {s}");
            }
            for americanism in ["color", "favorite", "canceled"] {
                assert!(!s.contains(americanism), "{f:?} copy is not UK English: {s}");
            }
            assert!(s.ends_with('.') || s.ends_with('?'), "{f:?} copy: {s}");
        }
    }

    // ─── The join flow ───

    struct Host {
        persona: Keys,
        runtime: Keys,
    }

    impl Host {
        fn new() -> Self {
            Host { persona: Keys::generate(), runtime: Keys::generate() }
        }
        async fn answer(&self, session: &str, accepted: bool, reason: Option<&str>) -> Answer {
            Answer {
                v: PAYLOAD_VERSION,
                session: session.to_string(),
                persona: npub_of(&self.persona.public_key()),
                attestation: crate::runtime_identity::mint_player_attestation(
                    &self.persona,
                    &self.runtime.public_key(),
                    nostr::Timestamp::from(NOW - 10),
                    90,
                )
                .await
                .unwrap(),
                accepted,
                reason: reason.map(str::to_string),
                protocol: crate::protocol::PROTOCOL_VERSION,
                candidates: if accepted {
                    vec![Candidate { kind: "lan".to_string(), addr: "127.0.0.1:9".to_string() }]
                } else {
                    vec![]
                },
                world_name: "Ivy's Hollow".to_string(),
                sent_at: NOW,
            }
        }
    }

    fn a_joiner(hub: &FakeRelayHub, host_runtime: [u8; 32]) -> OnlineJoin {
        let persona = Keys::generate();
        let runtime = Keys::generate();
        let attestation = rt()
            .block_on(crate::runtime_identity::mint_player_attestation(
                &persona,
                &runtime.public_key(),
                nostr::Timestamp::from(NOW - 10),
                90,
            ))
            .unwrap();
        let identity = RuntimeIdentity::from_parts(runtime, Some(attestation));
        let sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        OnlineJoin::start(
            identity,
            persona.public_key().to_bytes(),
            Box::new(hub.client()),
            sock,
            vec![Candidate { kind: "lan".to_string(), addr: "127.0.0.1:9".to_string() }],
            host_runtime,
            None,
            "Rowan".to_string(),
            NOW,
        )
        .unwrap()
    }

    #[test]
    fn starting_a_join_publishes_exactly_one_offer_to_the_host() {
        rt().block_on(async {
            let hub = FakeRelayHub::new();
            let host = Host::new();
            let host_relay = hub.client();
            host_relay
                .subscribe(KIND_JOIN_OFFER, &host.runtime.public_key().to_hex())
                .unwrap();
            let joiner = a_joiner(&hub, host.runtime.public_key().to_bytes());
            let ev = host_relay.try_recv().expect("the offer should have been published");
            let offer = crate::rendezvous::payload::open_offer(&host.runtime, &ev).unwrap();
            assert_eq!(offer.session, joiner.session());
            assert!(host_relay.try_recv().is_none(), "exactly one offer");
        });
    }

    #[test]
    fn no_answer_within_eight_seconds_is_the_didnt_answer_copy() {
        let hub = FakeRelayHub::new();
        let mut joiner = a_joiner(&hub, Keys::generate().public_key().to_bytes());
        assert_eq!(joiner.poll(NOW, std::time::Duration::from_secs(1)), JoinStep::Waiting);
        assert_eq!(
            joiner.poll(NOW, ANSWER_TIMEOUT),
            JoinStep::Failed(failure_copy(JoinFailure::NoAnswer, "Rowan"))
        );
    }

    #[test]
    fn a_protocol_mismatch_refusal_is_explained_to_the_player() {
        rt().block_on(async {
            let hub = FakeRelayHub::new();
            let host = Host::new();
            let mut joiner = a_joiner(&hub, host.runtime.public_key().to_bytes());
            let session = joiner.session().to_string();
            let answer = host.answer(&session, false, Some("protocol-mismatch")).await;
            let ev = seal_answer(
                &host.runtime,
                &nostr::PublicKey::from_slice(&joiner.runtime_pubkey()).unwrap(),
                &answer,
            )
            .await
            .unwrap();
            hub.client().publish(&ev).unwrap();
            assert_eq!(
                joiner.poll(NOW, std::time::Duration::from_secs(1)),
                JoinStep::Failed(failure_copy(JoinFailure::ProtocolMismatch, "Rowan"))
            );
        });
    }

    #[test]
    fn a_full_refusal_is_explained_to_the_player() {
        rt().block_on(async {
            let hub = FakeRelayHub::new();
            let host = Host::new();
            let mut joiner = a_joiner(&hub, host.runtime.public_key().to_bytes());
            let session = joiner.session().to_string();
            let answer = host.answer(&session, false, Some("full")).await;
            let ev = seal_answer(
                &host.runtime,
                &nostr::PublicKey::from_slice(&joiner.runtime_pubkey()).unwrap(),
                &answer,
            )
            .await
            .unwrap();
            hub.client().publish(&ev).unwrap();
            assert_eq!(
                joiner.poll(NOW, std::time::Duration::from_secs(1)),
                JoinStep::Failed(failure_copy(JoinFailure::Full, "Rowan"))
            );
        });
    }

    #[test]
    fn an_accepted_answer_moves_to_connecting_and_records_the_host_as_a_contact() {
        rt().block_on(async {
            let hub = FakeRelayHub::new();
            let host = Host::new();
            let mut joiner = a_joiner(&hub, host.runtime.public_key().to_bytes());
            let session = joiner.session().to_string();
            let answer = host.answer(&session, true, None).await;
            let ev = seal_answer(
                &host.runtime,
                &nostr::PublicKey::from_slice(&joiner.runtime_pubkey()).unwrap(),
                &answer,
            )
            .await
            .unwrap();
            hub.client().publish(&ev).unwrap();

            assert_eq!(
                joiner.poll(NOW, std::time::Duration::from_secs(1)),
                JoinStep::Connecting { world_name: "Ivy's Hollow".to_string() }
            );
            let contact = joiner.host_contact().expect("the host becomes a contact — mutually");
            assert_eq!(contact.pubkey, host.persona.public_key().to_bytes());
            assert_eq!(contact.tier, crate::comms::Tier::Kith);
            assert_eq!(contact.runtime_pubkey, Some(host.runtime.public_key().to_bytes()));
        });
    }

    #[test]
    fn an_answer_signed_by_the_wrong_key_is_ignored_and_the_wait_continues() {
        rt().block_on(async {
            let hub = FakeRelayHub::new();
            let host = Host::new();
            let impostor = Host::new();
            let mut joiner = a_joiner(&hub, host.runtime.public_key().to_bytes());
            let session = joiner.session().to_string();
            // A valid-looking answer for our session, from somebody else.
            let answer = impostor.answer(&session, true, None).await;
            let ev = seal_answer(
                &impostor.runtime,
                &nostr::PublicKey::from_slice(&joiner.runtime_pubkey()).unwrap(),
                &answer,
            )
            .await
            .unwrap();
            hub.client().publish(&ev).unwrap();
            assert_eq!(
                joiner.poll(NOW, std::time::Duration::from_secs(1)),
                JoinStep::Waiting,
                "an answer from anyone but the host we called is not an answer"
            );
        });
    }
}
```

- [ ] **Step 2: Run them to verify they fail**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo test --bin axenstax-engine online_join 2>&1 | tail -20
```
Expected: FAIL — `file not found for module online_join`.

- [ ] **Step 3: Write the implementation**

Put this above the test module in `game/engine/src/online_join.rs`:

```rust
//! Joining a friend's world.
//!
//! Publish one offer to the host's runtime key, wait up to eight seconds for an
//! answer, punch toward whatever addresses it names, race a QUIC connect across
//! them, and then run the ordinary authed join handshake on whichever won.
//!
//! Everything a player might see when this does not work lives in
//! [`failure_copy`] — four sentences, pinned verbatim by unit tests, written
//! for a child to read and act on.
//!
//! Spec: `docs/superpowers/specs/2026-09-06-online-play-by-contact-design.md`
//! §4.4, §5.2.
#![cfg(not(target_arch = "wasm32"))]

use std::net::UdpSocket;
use std::time::Duration;

use crate::contacts::{AddedVia, Contact};
use crate::nat::candidates::parse_addrs;
use crate::online_admission::{refusal_from_wire, Refusal};
use crate::rendezvous::payload::{
    npub_of, seal_offer, Candidate, Offer, KIND_JOIN_ANSWER, PAYLOAD_VERSION,
};
use crate::rendezvous::relay_client::RendezvousRelay;
use crate::rendezvous::verify::verify_answer;
use crate::runtime_identity::RuntimeIdentity;
use crate::transport::ClientTransport;

/// How long to wait for an answer before giving up. Same budget as the connect
/// race, so a failed join never takes more than about sixteen seconds end to
/// end.
pub const ANSWER_TIMEOUT: Duration = Duration::from_secs(8);

/// The four ways a join can fail in a way worth telling somebody about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoinFailure {
    /// No answer inside [`ANSWER_TIMEOUT`].
    NoAnswer,
    /// Answered, but no candidate completed a handshake.
    NoConnect,
    ProtocolMismatch,
    Full,
}

/// The player-facing message for a failure. Spec §4.4, verbatim.
///
/// Rules these four sentences follow, and that the tests enforce: no jargon a
/// child would have to look up, UK English, and — where there is something to
/// do about it — say what that is.
pub fn failure_copy(failure: JoinFailure, name: &str) -> String {
    match failure {
        JoinFailure::NoAnswer => {
            format!("{name} didn't answer. Are they online with the world open?")
        }
        JoinFailure::NoConnect => format!(
            "Couldn't reach {name}'s world. Their router needs UPnP turned on, or you both \
             need IPv6. Ask them to check Settings → Online in the game."
        ),
        JoinFailure::ProtocolMismatch => {
            "You're on different versions. One of you needs to update.".to_string()
        }
        JoinFailure::Full => format!("{name}'s world is full."),
    }
}

/// Where a join has got to. Returned from every `poll`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JoinStep {
    /// Offer sent, no answer yet.
    Waiting,
    /// Answered and accepted; the connect race is running.
    Connecting { world_name: String },
    /// A candidate won. Take the transport and run the authed handshake.
    Ready {
        world_name: String,
        host_persona: [u8; 32],
    },
    /// Give up and show this to the player.
    Failed(String),
}

enum Phase {
    Waiting,
    Connecting {
        world_name: String,
        connect: crate::network::OnlineConnect,
    },
    Done,
}

pub struct OnlineJoin {
    identity: RuntimeIdentity,
    relay: Box<dyn RendezvousRelay>,
    /// Held until an answer arrives, then moved into the connect race.
    socket: Option<UdpSocket>,
    session: String,
    host_runtime: [u8; 32],
    host_persona: Option<[u8; 32]>,
    display_name: String,
    host_contact: Option<Contact>,
    transport: Option<Box<dyn ClientTransport>>,
    phase: Phase,
}

impl OnlineJoin {
    /// Publish the offer and start waiting.
    #[allow(clippy::too_many_arguments)] // every one is a distinct dependency
    pub fn start(
        identity: RuntimeIdentity,
        persona: [u8; 32],
        relay: Box<dyn RendezvousRelay>,
        socket: UdpSocket,
        candidates: Vec<Candidate>,
        host_runtime: [u8; 32],
        bearer: Option<[u8; 16]>,
        display_name: String,
        now: u64,
    ) -> Result<OnlineJoin, String> {
        let attestation = identity
            .attestation()
            .cloned()
            .ok_or_else(|| "sign in and attest this device before playing online".to_string())?;
        let runtime_pk = identity.runtime_pubkey();
        relay.subscribe(KIND_JOIN_ANSWER, &runtime_pk.to_hex())?;

        let session = crate::rendezvous::payload::new_session_id();
        let offer = Offer {
            v: PAYLOAD_VERSION,
            session: session.clone(),
            persona: npub_of(
                &nostr::PublicKey::from_slice(&persona).map_err(|e| e.to_string())?,
            ),
            attestation,
            bearer: bearer.map(hex::encode),
            protocol: crate::protocol::PROTOCOL_VERSION,
            candidates,
            sent_at: now,
        };
        let recipient =
            nostr::PublicKey::from_slice(&host_runtime).map_err(|e| e.to_string())?;
        let keys = identity.keys().clone();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("no runtime: {e}"))?;
        let ev = rt.block_on(seal_offer(&keys, &recipient, &offer))?;
        relay.publish(&ev)?;

        Ok(OnlineJoin {
            identity,
            relay,
            socket: Some(socket),
            session,
            host_runtime,
            host_persona: None,
            display_name,
            host_contact: None,
            transport: None,
            phase: Phase::Waiting,
        })
    }

    pub fn session(&self) -> &str {
        &self.session
    }

    /// This joiner's runtime pubkey — what an answer must be addressed to.
    pub fn runtime_pubkey(&self) -> [u8; 32] {
        self.identity.runtime_pubkey().to_bytes()
    }

    /// The host, as a contact to fold into the mirror. `Some` once an accepted
    /// answer has been verified — the join is mutual, so calling somebody and
    /// being let in makes them a contact both ways.
    pub fn host_contact(&self) -> Option<Contact> {
        self.host_contact.clone()
    }

    /// The winning transport, once [`JoinStep::Ready`] has been returned. Taken
    /// once; the caller passes it to `RemoteClient::connect_authed_on_transport`.
    pub fn take_transport(&mut self) -> Option<Box<dyn ClientTransport>> {
        self.transport.take()
    }

    /// Advance. `elapsed` is time since `start`; `now` is unix seconds.
    pub fn poll(&mut self, now: u64, elapsed: Duration) -> JoinStep {
        match &mut self.phase {
            Phase::Done => JoinStep::Waiting,
            Phase::Waiting => {
                while let Some(ev) = self.relay.try_recv() {
                    let verified =
                        match verify_answer(&ev, self.identity.keys(), &self.session, now) {
                            Ok(v) => v,
                            Err(e) => {
                                log::debug!("[online] dropped an answer: {e:?}");
                                continue;
                            }
                        };
                    // An answer from anybody but the host we called is not an
                    // answer, however well-formed it is.
                    if verified.runtime != self.host_runtime {
                        log::debug!("[online] answer from an unexpected runtime key");
                        continue;
                    }
                    return self.on_answer(verified, now);
                }
                if elapsed >= ANSWER_TIMEOUT {
                    self.phase = Phase::Done;
                    return JoinStep::Failed(failure_copy(
                        JoinFailure::NoAnswer,
                        &self.display_name,
                    ));
                }
                JoinStep::Waiting
            }
            Phase::Connecting {
                world_name,
                connect,
            } => match connect.outcome.try_recv() {
                Ok(Ok(_addr)) => {
                    let world_name = world_name.clone();
                    let host_persona = self.host_persona.unwrap_or([0u8; 32]);
                    self.phase = Phase::Done;
                    JoinStep::Ready {
                        world_name,
                        host_persona,
                    }
                }
                Ok(Err(e)) => {
                    log::info!("[online] connect race lost: {e}");
                    self.phase = Phase::Done;
                    JoinStep::Failed(failure_copy(JoinFailure::NoConnect, &self.display_name))
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => JoinStep::Connecting {
                    world_name: world_name.clone(),
                },
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.phase = Phase::Done;
                    JoinStep::Failed(failure_copy(JoinFailure::NoConnect, &self.display_name))
                }
            },
        }
    }

    fn on_answer(
        &mut self,
        verified: crate::rendezvous::verify::VerifiedAnswer,
        now: u64,
    ) -> JoinStep {
        let answer = verified.answer;
        if !answer.accepted {
            self.phase = Phase::Done;
            let failure = match answer.reason.as_deref().and_then(refusal_from_wire) {
                Some(Refusal::ProtocolMismatch) => JoinFailure::ProtocolMismatch,
                Some(Refusal::Full) => JoinFailure::Full,
                // Anything else should never have been sent; treat it as
                // "they're not there for you" rather than inventing new copy.
                _ => JoinFailure::NoAnswer,
            };
            return JoinStep::Failed(failure_copy(failure, &self.display_name));
        }

        self.host_persona = Some(verified.persona);
        self.host_contact = Some(Contact {
            pubkey: verified.persona,
            display_name: Some(self.display_name.clone()),
            tier: crate::comms::Tier::Kith,
            is_child: false,
            runtime_pubkey: Some(verified.runtime),
            added_via: AddedVia::Invite,
            added_at: now,
        });

        let Some(socket) = self.socket.take() else {
            self.phase = Phase::Done;
            return JoinStep::Failed(failure_copy(JoinFailure::NoConnect, &self.display_name));
        };
        let addrs = parse_addrs(&answer.candidates);
        let connect = crate::network::connect_to_server_on_socket(
            socket,
            addrs,
            self.session.clone(),
        );
        // The transport is usable the moment the race wins; hold it here so the
        // caller takes it in one piece at `Ready`.
        self.transport = Some(Box::new(std::mem::replace(
            &mut { connect },
            crate::network::connect_to_server_on_socket(
                UdpSocket::bind("127.0.0.1:0").expect("loopback bind"),
                Vec::new(),
                String::new(),
            ),
        )
        .transport));
        // NOTE: replace the two lines above with the straightforward move once
        // `OnlineConnect` is destructured — see Step 4.
        JoinStep::Connecting {
            world_name: answer.world_name,
        }
    }
}
```

- [ ] **Step 4: Replace the awkward transport move with a clean destructure**

The block flagged by the `NOTE` above is deliberately wrong-looking — fix it now
rather than leaving it. Replace the whole tail of `on_answer` (from `let addrs =`
to the `JoinStep::Connecting` return) with:

```rust
        let addrs = parse_addrs(&answer.candidates);
        let crate::network::OnlineConnect { transport, outcome } =
            crate::network::connect_to_server_on_socket(socket, addrs, self.session.clone());
        self.transport = Some(Box::new(transport));
        let world_name = answer.world_name;
        self.phase = Phase::Connecting {
            world_name: world_name.clone(),
            connect: OutcomeChannel { outcome },
        };
        JoinStep::Connecting { world_name }
```

and change `Phase::Connecting`'s payload plus add the tiny holder, so the phase
carries only the channel (the transport is already parked on `self`):

```rust
/// Just the race's outcome channel — the transport it came with is parked on
/// `OnlineJoin` so the caller can take it in one piece.
struct OutcomeChannel {
    outcome: std::sync::mpsc::Receiver<Result<std::net::SocketAddr, String>>,
}

enum Phase {
    Waiting,
    Connecting {
        world_name: String,
        connect: OutcomeChannel,
    },
    Done,
}
```

and in `poll`'s `Phase::Connecting` arm, read `connect.outcome.try_recv()` (the
arm's body already does).

- [ ] **Step 5: Register the module**

In `game/engine/src/main.rs`:

```rust
// Online play by contact — joiner-side orchestration + the failure copy.
#[cfg(not(target_arch = "wasm32"))]
mod online_join;
```

- [ ] **Step 6: Run the tests to verify they pass**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo test --bin axenstax-engine online_join 2>&1 | tail -20
```
Expected: PASS — 11 passed. The five copy tests are the ones that must never be
edited without a decision: they pin spec §4.4 word for word.

- [ ] **Step 7: Verify clippy**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo clippy -- -D warnings 2>&1 | tail -20
```
Expected: no warnings.

- [ ] **Step 8: Commit**

```bash
git add game/engine/src/online_join.rs game/engine/src/main.rs && git commit -m "$(cat <<'EOF'
feat(online): joiner-side orchestration + the four failure messages (P4)

One offer, an 8s wait, then punch and race. The §4.4 copy is verbatim and
pinned by tests that also forbid jargon (NAT, STUN, QUIC, npub, socket,
candidate, relay) and Americanisms — these four sentences are the whole of what
a child sees when this doesn't work. An answer from anybody but the host we
called is ignored and the wait continues.

Claude-Session: https://claude.ai/code/session_011gW6q5QFBTbF1BNMHiayce
EOF
)" -- game/engine/src/online_join.rs game/engine/src/main.rs
```

---

### Task 17: Friends & servers column, Host online, Online panel (`friends_ui.rs`, `menu.rs`)

**Files:**
- Modify: `game/engine/src/friends_ui.rs`
- Modify: `game/engine/src/menu.rs` (`MenuAction`, `MenuDialog`, `CardAction`, the column mount, the world card)

**Interfaces:**
- Consumes: `contacts::{Contact, AddedVia, load_local_book, upsert, save_mirror, mirror_path}`,
  `online_admission::admits_play`, `invite::{Invite, InviteError}`, `online_host::OnlineHost`.
- Produces:
  ```rust
  // menu.rs
  pub enum MenuAction { /* … existing … */
      #[cfg(not(target_arch = "wasm32"))] HostWorldOnline(String),
      #[cfg(not(target_arch = "wasm32"))] JoinContact { persona_hex: String, display_name: String },
      #[cfg(not(target_arch = "wasm32"))] JoinByInvite(String),
      #[cfg(not(target_arch = "wasm32"))] AddFriendNpub(String),
  }
  pub enum MenuDialog { /* … existing … */
      #[cfg(not(target_arch = "wasm32"))] AddFriend { input: String, error: Option<String> },
  }
  enum CardAction { /* … existing … */ #[cfg(not(target_arch = "wasm32"))] HostOnline }
  pub struct MenuState { /* … existing … */
      #[cfg(not(target_arch = "wasm32"))] pub contacts: Vec<crate::contacts::Contact>,
  }
  // friends_ui.rs
  pub enum AddFriendInput { Invite(Box<crate::invite::Invite>), Npub(String) }
  pub fn parse_add_friend(text: &str, now: u64) -> Result<AddFriendInput, String>;
  pub fn contact_row_label(c: &Contact) -> String;
  pub fn can_join(c: &Contact) -> bool;
  pub fn draw_friends_column(ui: &mut egui::Ui, state: &mut MenuState) -> MenuAction;
  pub struct OnlinePanelView<'a> {
      pub invite_link: &'a str, pub candidates: &'a [crate::rendezvous::payload::Candidate],
      pub relays: (usize, usize), pub warning: Option<&'a str>,
  }
  pub fn reachability_summary(candidates: &[crate::rendezvous::payload::Candidate]) -> String;
  pub fn draw_online_panel(ui: &mut egui::Ui, view: &OnlinePanelView);
  ```

- [ ] **Step 1: Write the failing tests**

Append to the `mod tests` block in `game/engine/src/friends_ui.rs`:

```rust
    use crate::comms::Tier;
    use crate::contacts::{AddedVia, Contact};
    use crate::rendezvous::payload::Candidate;

    fn contact(tier: Tier, name: Option<&str>) -> Contact {
        Contact {
            pubkey: [3u8; 32],
            display_name: name.map(str::to_string),
            tier,
            is_child: false,
            runtime_pubkey: None,
            added_via: AddedVia::Kenspeckle,
            added_at: 0,
        }
    }

    #[test]
    fn only_kin_and_kith_get_a_join_button() {
        // The column SHOWS ken (they are somebody you recognise) but does not
        // offer to take you into their world — the same rule the host applies.
        assert!(can_join(&contact(Tier::Kin, Some("Mum"))));
        assert!(can_join(&contact(Tier::Kith, Some("Rowan"))));
        assert!(!can_join(&contact(Tier::Ken, Some("Someone"))));
        assert!(!can_join(&contact(Tier::Stranger, None)));
    }

    #[test]
    fn a_nameless_contact_falls_back_to_a_short_npub_never_hex() {
        let label = contact_row_label(&contact(Tier::Kith, None));
        assert!(label.starts_with("npub1"), "must render as an npub: {label}");
        assert!(label.contains('…'));
    }

    #[test]
    fn a_named_contact_shows_its_name() {
        assert_eq!(contact_row_label(&contact(Tier::Kin, Some("Mum"))), "Mum");
    }

    #[test]
    fn add_friend_accepts_an_invite_link() {
        let inv = crate::invite::Invite {
            host_persona: "npub1sg6plzptd64u62a878hep2kev88swjh3tw00gjsfl8f237lmu63q0uf63m"
                .to_string(),
            host_runtime: [0xab; 32],
            relays: vec!["wss://nos.lol".to_string()],
            bearer: [1u8; 16],
            expires_at: 2_000_000_000,
            world_name: "Ivy's Hollow".to_string(),
        };
        match parse_add_friend(&inv.to_link(), 1_000_000_000) {
            Ok(AddFriendInput::Invite(got)) => assert_eq!(*got, inv),
            other => panic!("expected an invite, got {other:?}"),
        }
    }

    #[test]
    fn add_friend_accepts_a_bare_npub() {
        let npub = "npub1sg6plzptd64u62a878hep2kev88swjh3tw00gjsfl8f237lmu63q0uf63m";
        match parse_add_friend(&format!("  {npub}  "), 0) {
            Ok(AddFriendInput::Npub(got)) => assert_eq!(got, npub),
            other => panic!("expected an npub, got {other:?}"),
        }
    }

    #[test]
    fn add_friend_refuses_hex_and_says_why() {
        // Hex is internal only ([[feedback_npub_only_display]]); accepting it
        // here would teach players to pass it around.
        let err = parse_add_friend(&"ab".repeat(32), 0).unwrap_err();
        assert!(err.contains("npub"), "{err}");
    }

    #[test]
    fn add_friend_explains_an_expired_invite_in_plain_words() {
        let inv = crate::invite::Invite {
            host_persona: "npub1sg6plzptd64u62a878hep2kev88swjh3tw00gjsfl8f237lmu63q0uf63m"
                .to_string(),
            host_runtime: [0xab; 32],
            relays: vec!["wss://nos.lol".to_string()],
            bearer: [1u8; 16],
            expires_at: 100,
            world_name: "W".to_string(),
        };
        let err = parse_add_friend(&inv.to_link(), 200).unwrap_err();
        assert!(err.contains("expired"), "{err}");
        assert!(!err.contains("InviteError"), "no debug formatting: {err}");
    }

    #[test]
    fn add_friend_refuses_empty_input() {
        assert!(parse_add_friend("   ", 0).is_err());
    }

    #[test]
    fn the_reachability_summary_names_places_not_protocols() {
        let s = reachability_summary(&[
            Candidate { kind: "lan".to_string(), addr: "192.168.1.2:7700".to_string() },
            Candidate { kind: "upnp".to_string(), addr: "198.51.100.7:7700".to_string() },
            Candidate { kind: "stun".to_string(), addr: "203.0.113.9:41000".to_string() },
        ]);
        assert_eq!(s, "Reachable: home network, router mapping, internet");
        assert!(!s.contains("192.168"), "a summary must not leak addresses: {s}");
    }

    #[test]
    fn the_reachability_summary_says_so_when_only_the_home_network_works() {
        let s = reachability_summary(&[Candidate {
            kind: "lan".to_string(),
            addr: "192.168.1.2:7700".to_string(),
        }]);
        assert_eq!(s, "Reachable: home network");
    }

    #[test]
    fn the_reachability_summary_handles_nothing_at_all() {
        assert_eq!(reachability_summary(&[]), "Reachable: nothing yet");
    }
```

- [ ] **Step 2: Run them to verify they fail**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo test --bin axenstax-engine friends_ui 2>&1 | tail -20
```
Expected: FAIL — `cannot find function can_join`.

- [ ] **Step 3: Add the pure helpers**

Append to `game/engine/src/friends_ui.rs` (above the test module):

```rust
use crate::contacts::Contact;
use crate::rendezvous::payload::Candidate;

/// What the player typed into "Add a friend".
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AddFriendInput {
    /// A full invite link — adds a pending contact and starts a join at once.
    Invite(Box<crate::invite::Invite>),
    /// A bare npub — adds a contact you can call later, once they are hosting.
    Npub(String),
}

/// Parse the paste box. Returns a message written for a person, not a debug
/// dump, because whatever comes back goes straight under the field.
pub fn parse_add_friend(text: &str, now: u64) -> Result<AddFriendInput, String> {
    let t = text.trim();
    if t.is_empty() {
        return Err("Paste an invite link or a friend's npub.".to_string());
    }
    if t.starts_with("axenstax://") {
        return crate::invite::Invite::parse(t, now)
            .map(|i| AddFriendInput::Invite(Box::new(i)))
            .map_err(|e| e.to_string());
    }
    if t.starts_with("npub1") && nostr::PublicKey::parse(t).is_ok() {
        return Ok(AddFriendInput::Npub(t.to_string()));
    }
    Err("That doesn't look like an invite link or an npub. An address starts with \
         'npub1'."
        .to_string())
}

/// Whether this contact is close enough to offer a Join button. Exactly the
/// host's rule (`online_admission::admits_play`), so the button is never
/// offered for a join that would be refused.
pub fn can_join(c: &Contact) -> bool {
    crate::online_admission::admits_play(c.tier)
}

/// The row label: their name if the book has one, otherwise a short npub.
/// Never hex — [[feedback_npub_only_display]].
pub fn contact_row_label(c: &Contact) -> String {
    if let Some(n) = &c.display_name
        && !n.trim().is_empty()
    {
        return n.clone();
    }
    let npub = nostr::PublicKey::from_slice(&c.pubkey)
        .ok()
        .and_then(|pk| nostr::ToBech32::to_bech32(&pk).ok())
        .unwrap_or_else(|| "npub1…".to_string());
    short_npub(&npub)
}

/// A one-line, address-free summary of how reachable this host is.
///
/// Places, not protocols: "home network" rather than "LAN", "router mapping"
/// rather than "UPnP", "internet" rather than "STUN reflexive". And no
/// addresses at all — this line is on screen while somebody streams.
pub fn reachability_summary(candidates: &[Candidate]) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for (kind, label) in [
        ("lan", "home network"),
        ("v6", "direct internet address"),
        ("upnp", "router mapping"),
        ("stun", "internet"),
    ] {
        if candidates.iter().any(|c| c.kind == kind) {
            parts.push(label);
        }
    }
    if parts.is_empty() {
        return "Reachable: nothing yet".to_string();
    }
    format!("Reachable: {}", parts.join(", "))
}

/// Everything the Online panel needs to draw. A view struct so the panel can be
/// rendered from the lobby or from the in-game `/online` overlay without either
/// reaching into `OnlineHost`.
pub struct OnlinePanelView<'a> {
    pub invite_link: &'a str,
    pub candidates: &'a [Candidate],
    pub relays: (usize, usize),
    pub warning: Option<&'a str>,
}

/// QR + copy button + reachability + relay count + any warning.
pub fn draw_online_panel(ui: &mut egui::Ui, view: &OnlinePanelView) {
    ui.label(egui::RichText::new("Playing online").size(16.0).strong());
    ui.add_space(4.0);
    crate::menu::draw_qr(ui, view.invite_link, 148.0);
    ui.add_space(4.0);
    if ui.button("Copy invite link").clicked() {
        ui.ctx().copy_text(view.invite_link.to_string());
    }
    ui.add_space(6.0);
    ui.label(
        egui::RichText::new(reachability_summary(view.candidates))
            .size(11.0)
            .color(egui::Color32::from_rgb(160, 175, 160)),
    );
    let (up, total) = view.relays;
    ui.label(
        egui::RichText::new(format!("Relays connected: {up}/{total}"))
            .size(11.0)
            .color(egui::Color32::from_rgb(150, 150, 160)),
    );
    if let Some(w) = view.warning {
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(w)
                .size(11.0)
                .color(egui::Color32::from_rgb(230, 190, 120)),
        );
    }
}

/// The "Friends & servers" column: your address, the people you know, the paste
/// box, then the saved-servers list.
///
/// This lists **people you already know**. There is nothing to browse and
/// nobody new to find here — that is the point (CLAUDE.md red lines 1 and 4).
pub fn draw_friends_column(ui: &mut egui::Ui, state: &mut crate::menu::MenuState) -> MenuAction {
    let mut action = MenuAction::None;

    draw_your_address(ui, crate::signet::native_signer::load_identity().npub().as_deref());
    ui.add_space(10.0);
    ui.separator();
    ui.add_space(6.0);

    ui.label(egui::RichText::new("Friends").size(16.0).strong());
    ui.label(
        egui::RichText::new("People you know, to build with.")
            .size(11.0)
            .color(egui::Color32::from_rgb(150, 150, 160)),
    );
    ui.add_space(4.0);

    if state.contacts.is_empty() {
        ui.label(
            egui::RichText::new(
                "Nobody yet. Paste a friend's invite link below, or send them your address.",
            )
            .size(11.0)
            .color(egui::Color32::from_rgb(150, 150, 160)),
        );
    }

    // Snapshot so the row loop can stage an action without holding a borrow.
    let rows: Vec<(String, String, bool)> = state
        .contacts
        .iter()
        .map(|c| {
            (
                hex::encode(c.pubkey),
                contact_row_label(c),
                can_join(c),
            )
        })
        .collect();
    for (persona_hex, label, joinable) in rows {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(&label).size(13.0));
            if joinable && ui.small_button("Join").clicked() {
                action = MenuAction::JoinContact {
                    persona_hex: persona_hex.clone(),
                    display_name: label.clone(),
                };
            }
        });
    }

    ui.add_space(8.0);
    if ui.button("Add a friend").clicked() {
        state.dialog = crate::menu::MenuDialog::AddFriend {
            input: String::new(),
            error: None,
        };
    }

    action
}
```

Add `use crate::menu::MenuAction;` to the file's imports.

- [ ] **Step 4: Wire the menu types**

In `game/engine/src/menu.rs`:

(a) add to `MenuAction`:

```rust
    /// Online play by contact — host this world for friends in other houses.
    /// Native only.
    #[cfg(not(target_arch = "wasm32"))]
    HostWorldOnline(String),
    /// Call a contact who is hosting. `persona_hex` is internal; the display
    /// name is what any failure message will use.
    #[cfg(not(target_arch = "wasm32"))]
    JoinContact {
        persona_hex: String,
        display_name: String,
    },
    /// A pasted invite link: adds a pending Kith contact and calls at once.
    #[cfg(not(target_arch = "wasm32"))]
    JoinByInvite(String),
    /// A pasted npub: adds a contact to call later.
    #[cfg(not(target_arch = "wasm32"))]
    AddFriendNpub(String),
```

(b) add to `MenuDialog`:

```rust
    /// Online play by contact — paste an invite link or an npub.
    #[cfg(not(target_arch = "wasm32"))]
    AddFriend { input: String, error: Option<String> },
```

(c) add to `CardAction` and `handle_card_action`:

```rust
    /// Native only — host this world for friends outside the house.
    #[cfg(not(target_arch = "wasm32"))]
    HostOnline,
```
```rust
        #[cfg(not(target_arch = "wasm32"))]
        CardAction::HostOnline => {
            return MenuAction::HostWorldOnline(state.worlds[idx].folder_name.clone());
        }
```

(d) in `draw_world_card`, beside the existing Host button (line ~3266):

```rust
                #[cfg(not(target_arch = "wasm32"))]
                if action_button(ui, "Host online").clicked() {
                    card_action = CardAction::HostOnline;
                }
```

(e) add the contacts field to `MenuState` and load it in `new`:

```rust
    /// Online play by contact — the local address book, loaded once for the
    /// Friends column. Native only.
    #[cfg(not(target_arch = "wasm32"))]
    pub contacts: Vec<crate::contacts::Contact>,
```
```rust
            #[cfg(not(target_arch = "wasm32"))]
            contacts: crate::contacts::load_local_book(),
```

(f) swap the column: in the `my_servers_column` panel block (line ~2634),
replace `draw_my_servers_column(ui, state)` with
`crate::friends_ui::draw_friends_column(ui, state)`, rename the panel id to
`"friends_column"`, and — because `draw_friends_column` now draws "Your address"
itself — **remove** the "Your address" block that Task 5 inserted into
`draw_my_servers_column`. Keep `draw_my_servers_column` and call it from the
bottom of `draw_friends_column`:

```rust
    ui.add_space(10.0);
    ui.separator();
    let servers_action = draw_my_servers_column_inner(ui, state);
    if !matches!(servers_action, MenuAction::None) {
        action = servers_action;
    }
```
(rename `draw_my_servers_column` to `pub(crate) fn draw_my_servers_column_inner`
and drop its own outer `ScrollArea`, since the Friends column already scrolls.)

(g) render the AddFriend dialog next to the other `MenuDialog` arms:

```rust
        #[cfg(not(target_arch = "wasm32"))]
        MenuDialog::AddFriend { input, error } => {
            let mut close = false;
            let mut submit: Option<String> = None;
            egui::Window::new("Add a friend")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label("Paste their invite link, or their address (npub).");
                    ui.add(egui::TextEdit::singleline(input).desired_width(420.0));
                    if let Some(e) = error.as_ref() {
                        ui.colored_label(egui::Color32::from_rgb(230, 150, 150), e);
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Add").clicked() {
                            submit = Some(input.clone());
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                });
            if let Some(text) = submit {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                match crate::friends_ui::parse_add_friend(&text, now) {
                    Ok(crate::friends_ui::AddFriendInput::Invite(_)) => {
                        action = MenuAction::JoinByInvite(text);
                        close = true;
                    }
                    Ok(crate::friends_ui::AddFriendInput::Npub(npub)) => {
                        action = MenuAction::AddFriendNpub(npub);
                        close = true;
                    }
                    Err(e) => *error = Some(e),
                }
            }
            if close {
                state.dialog = MenuDialog::None;
            }
        }
```

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo test --bin axenstax-engine friends_ui 2>&1 | tail -20
```
Expected: PASS — 15 passed (4 from Task 5 plus 11 new).

- [ ] **Step 6: Verify clippy, the wasm build, and the lobby render**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo clippy -- -D warnings 2>&1 | tail -20
```
Expected: no warnings — every new `MenuAction`/`MenuDialog`/`CardAction` variant
is `#[cfg]`-gated, so the wasm `match` arms must still be exhaustive:
```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 trunk build 2>&1 | tail -10
```
Expected: `success`. Then:
```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo run -- --shot-lobby 2>&1 | tail -5
```
Expected: the screenshot shows "Your address", a Friends section, "Add a
friend", then My Servers.

- [ ] **Step 7: Commit**

```bash
git add game/engine/src/friends_ui.rs game/engine/src/menu.rs && git commit -m "$(cat <<'EOF'
feat(online): Friends & servers column, Host online, Online panel (P4)

The column lists people you already know — nothing to browse, nobody new to
find. Join is offered only where the host would actually admit you (the same
admits_play rule), so the button never promises a refusal. The reachability line
says places, not protocols, and carries no addresses. Add-a-friend takes an
invite link or an npub and refuses hex with a reason.

Claude-Session: https://claude.ai/code/session_011gW6q5QFBTbF1BNMHiayce
EOF
)" -- game/engine/src/friends_ui.rs game/engine/src/menu.rs
```

---

### Task 18: Game-loop dispatch and the `/online` command

**Files:**
- Modify: `game/engine/src/game_loop.rs` (the `MenuAction` match at ~7159–7200; a new `bind_and_gather` helper; per-tick polling)
- Create: `game/engine/src/commands/builtins/online.rs`
- Modify: `game/engine/src/commands/builtins/mod.rs`, `game/engine/src/commands/dispatch.rs`

**Interfaces:**
- Consumes: `online_host::{OnlineHost, HostEvent, unreachable_warning}`,
  `online_join::{OnlineJoin, JoinStep}`, `nat::candidates::{gather, reachable_beyond_lan}`,
  `nat::upnp::{map_port, PortMapping, SEARCH_TIMEOUT}`,
  `hosted_server::HostedServer::{start_online, set_access_policy}`,
  `remote_client::RemoteClient::connect_authed_on_transport`,
  `game_loop::native_join_sign_driver` (already exists, `src/game_loop.rs:111`),
  `graphics_settings::GraphicsSettings::{load, online_relays, online_port}`,
  `rendezvous::relay_client::MultiRelay`, `runtime_identity::RuntimeIdentity`,
  `signet::native_signer::{current_owner_pubkey, restore_signer}`.
- Produces:
  ```rust
  // game_loop.rs
  fn bind_and_gather(port: u16) -> Result<(std::net::UdpSocket, std::net::UdpSocket,
      Vec<crate::rendezvous::payload::Candidate>, Option<crate::nat::upnp::PortMapping>), String>;
  // GameState gains:
  //   online_host: Option<crate::online_host::OnlineHost>
  //   online_join: Option<(crate::online_join::OnlineJoin, std::time::Instant)>
  //   upnp_mapping: Option<crate::nat::upnp::PortMapping>
  //   show_online_panel: bool
  // commands/dispatch.rs
  pub enum CommandResult { /* … */ OnlineStatus, OnlineCopyInvite }
  ```

- [ ] **Step 1: Write the failing test for `/online`**

Create `game/engine/src/commands/builtins/online.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::registry::Command;

    #[test]
    fn the_command_is_named_online_and_is_not_a_cheat() {
        let c = OnlineCommand;
        assert_eq!(c.name(), "online");
        assert!(!c.is_cheat(), "showing your own invite link is not a cheat");
    }

    #[test]
    fn its_usage_names_both_subcommands() {
        let c = OnlineCommand;
        assert!(c.usage().contains("/online"));
        assert!(c.usage().contains("copy"));
    }

    #[test]
    fn no_argument_asks_for_status_and_copy_asks_to_copy() {
        // `execute` is exercised through the enum it returns, which is what the
        // game loop acts on — CommandContext is too wide to reach a
        // HostedServer, so every server-touching command works this way
        // (see /room, /spawncart).
        let c = OnlineCommand;
        assert!(matches!(c.result_for(&[]), CommandResult::OnlineStatus));
        assert!(matches!(
            c.result_for(&["copy".to_string()]),
            CommandResult::OnlineCopyInvite
        ));
    }

    #[test]
    fn an_unknown_subcommand_is_an_error_not_a_silent_status() {
        assert!(matches!(
            OnlineCommand.result_for(&["wat".to_string()]),
            CommandResult::Error(_)
        ));
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo test --bin axenstax-engine builtins::online 2>&1 | tail -20
```
Expected: FAIL — `file not found for module online`.

- [ ] **Step 3: Write the command**

Put this above the test module in `game/engine/src/commands/builtins/online.rs`:

```rust
//! `/online` — show this world's invite link and how reachable it is, or copy
//! the link to the clipboard.
//!
//! Like every other command that needs the `HostedServer` (`/room`,
//! `/spawncart`), this one only decides WHAT to do and returns it as a
//! [`CommandResult`]; the game loop, which owns `self.online_host`, does it.
//! `CommandContext` is deliberately too narrow to reach the server.
//!
//! Registered NATIVE ONLY: there is no online play on the web taster.

use crate::commands::dispatch::{CommandContext, CommandResult};
use crate::commands::registry::{Command, OpLevel};

pub struct OnlineCommand;

impl OnlineCommand {
    /// The decision, split out so it is testable without a `CommandContext`.
    pub fn result_for(&self, args: &[String]) -> CommandResult {
        match args.first().map(|s| s.to_lowercase()).as_deref() {
            None => CommandResult::OnlineStatus,
            Some("copy") => CommandResult::OnlineCopyInvite,
            Some(other) => CommandResult::Error(format!(
                "unknown /online subcommand '{other}' — usage: /online | /online copy"
            )),
        }
    }
}

impl Command for OnlineCommand {
    fn name(&self) -> &'static str {
        "online"
    }
    fn help(&self) -> &'static str {
        "Show your invite link and whether friends can reach you"
    }
    fn usage(&self) -> &'static str {
        "/online | /online copy"
    }
    fn min_op_level(&self) -> OpLevel {
        // Whoever is hosting may see their own invite; it is theirs.
        OpLevel::Op
    }
    fn is_cheat(&self) -> bool {
        false
    }
    fn execute(&self, _ctx: &mut CommandContext, args: &[String]) -> CommandResult {
        self.result_for(args)
    }
}
```

Add the two result variants in `game/engine/src/commands/dispatch.rs`, beside
`RoomInvite`:

```rust
    /// `/online` — the game loop prints the invite link, the reachability
    /// summary and the relay count into chat, and opens the Online panel.
    OnlineStatus,
    /// `/online copy` — the game loop puts the invite link on the clipboard.
    OnlineCopyInvite,
```

Register it in `game/engine/src/commands/builtins/mod.rs`:

```rust
// Online play by contact — native only (no online play on the web taster).
#[cfg(not(target_arch = "wasm32"))]
pub mod online;
```
```rust
    #[cfg(not(target_arch = "wasm32"))]
    registry.register(Box::new(online::OnlineCommand));
```

- [ ] **Step 4: Run the command tests to verify they pass**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo test --bin axenstax-engine builtins::online 2>&1 | tail -20
```
Expected: PASS — 4 passed. Every other `match` over `CommandResult` must now
handle the two new variants; clippy will name them if any is missed.

- [ ] **Step 5: Add the socket/candidate helper to `game_loop.rs`**

Near `native_join_sign_driver` (line ~111):

```rust
/// Bind the online-play socket and work out how the world can be reached.
///
/// Returns `(socket for quinn, clone for punching, candidates, upnp mapping)`.
/// The clone is what makes host-side punching possible at all: quinn takes
/// ownership of the socket, and a punch must leave from the same source port
/// the peer was told to expect (spec §4.3).
///
/// Blocking for up to ~3 s (UPnP search + two STUN round-trips), so call it on
/// a worker thread, never inside a frame.
#[cfg(not(target_arch = "wasm32"))]
fn bind_and_gather(
    port: u16,
) -> Result<
    (
        std::net::UdpSocket,
        std::net::UdpSocket,
        Vec<crate::rendezvous::payload::Candidate>,
        Option<crate::nat::upnp::PortMapping>,
    ),
    String,
> {
    let socket = std::net::UdpSocket::bind(("0.0.0.0", port))
        .map_err(|e| format!("could not bind UDP port {port}: {e}"))?;
    let local = socket
        .local_addr()
        .map_err(|e| format!("bound socket has no address: {e}"))?;

    // The router mapping is asked for first, so its external address is in the
    // candidate list gather() builds.
    let mapping = match crate::nat::candidates::local_outbound_v4() {
        Some(v4) => {
            let internal = std::net::SocketAddr::new(std::net::IpAddr::V4(v4), local.port());
            match crate::nat::upnp::map_port(internal, crate::nat::upnp::SEARCH_TIMEOUT) {
                Ok(m) => Some(m),
                Err(e) => {
                    log::info!("[online] no router mapping: {e}");
                    None
                }
            }
        }
        None => None,
    };

    let candidates = crate::nat::candidates::gather(
        &socket,
        mapping.as_ref().map(|m| m.external_addr()),
        std::time::Duration::from_secs(1),
    );
    let punch_socket = socket
        .try_clone()
        .map_err(|e| format!("could not clone the socket for punching: {e}"))?;
    Ok((socket, punch_socket, candidates, mapping))
}
```

- [ ] **Step 6: Add the state fields and the dispatch arms**

On `GameState` (native-gated fields):

```rust
    /// Online play by contact — the host-side rendezvous, while hosting online.
    #[cfg(not(target_arch = "wasm32"))]
    online_host: Option<crate::online_host::OnlineHost>,
    /// A join in progress, with the instant it started (for the 8 s wait).
    #[cfg(not(target_arch = "wasm32"))]
    online_join: Option<(crate::online_join::OnlineJoin, std::time::Instant)>,
    /// The router mapping to renew hourly and release when hosting stops.
    #[cfg(not(target_arch = "wasm32"))]
    upnp_mapping: Option<crate::nat::upnp::PortMapping>,
    /// Whether `/online` has the panel open.
    #[cfg(not(target_arch = "wasm32"))]
    show_online_panel: bool,
```
(initialise all four to `None`/`false` in `GameState::new`.)

Add the four dispatch arms beside `MenuAction::HostWorld` (~line 7159):

```rust
                #[cfg(not(target_arch = "wasm32"))]
                crate::menu::MenuAction::HostWorldOnline(folder_name) => {
                    let settings = crate::graphics_settings::GraphicsSettings::load();
                    let now = unix_now();
                    let started = (|| -> Result<_, String> {
                        let identity = crate::runtime_identity::RuntimeIdentity::load()?
                            .ok_or_else(|| "no runtime identity".to_string())?;
                        let persona = crate::signet::native_signer::current_owner_pubkey()
                            .and_then(|h| hex::decode(h).ok())
                            .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())
                            .ok_or_else(|| {
                                "Sign in with your Signet persona to host online.".to_string()
                            })?;
                        if identity.persona(nostr::Timestamp::from(now)).is_none() {
                            return Err(
                                "This device isn't linked to your persona yet — approve it on \
                                 your phone from Settings → Online."
                                    .to_string(),
                            );
                        }
                        let (socket, punch_socket, candidates, mapping) =
                            bind_and_gather(settings.online_port)?;
                        let host_seed = crate::save::load_world_meta(&folder_name).seed;
                        let mut server = crate::hosted_server::HostedServer::start_online(
                            1,
                            folder_name.clone(),
                            host_seed,
                            4,
                            socket,
                        )?;
                        let book = crate::contacts::load_local_book();
                        // Contacts at Kin/Kith are on the allowlist from the
                        // start; bearer admissions are appended as they happen.
                        let seed_allow: Vec<[u8; 32]> = book
                            .iter()
                            .filter(|c| crate::online_admission::admits_play(c.tier))
                            .map(|c| c.pubkey)
                            .collect();
                        server.set_access_policy(true, seed_allow, Vec::new());
                        let relay = crate::rendezvous::relay_client::MultiRelay::start(
                            settings.online_relays.clone(),
                        );
                        let host = crate::online_host::OnlineHost::start(
                            identity,
                            persona,
                            Box::new(relay),
                            punch_socket,
                            candidates,
                            folder_name.clone(),
                            settings.online_relays.clone(),
                            book,
                            now,
                        )?;
                        Ok((server, host, mapping))
                    })();

                    match started {
                        Ok((server, host, mapping)) => {
                            let link = host.invite_link();
                            let warn = !crate::nat::candidates::reachable_beyond_lan(
                                host.candidates(),
                            );
                            self.hosted_server = Some(server);
                            self.online_host = Some(host);
                            self.upnp_mapping = mapping;
                            self.toast = Some((
                                if warn {
                                    crate::online_host::unreachable_warning().to_string()
                                } else {
                                    "Invite link copied — send it to your friend.".to_string()
                                },
                                Instant::now() + std::time::Duration::from_secs(8),
                            ));
                            self.pending_clipboard = Some(link);
                            self.world_name = folder_name.clone();
                            load_world = Some(folder_name);
                        }
                        Err(e) => {
                            log::error!("[online] could not host: {e}");
                            self.toast = Some((
                                e,
                                Instant::now() + std::time::Duration::from_secs(8),
                            ));
                        }
                    }
                }
                #[cfg(not(target_arch = "wasm32"))]
                crate::menu::MenuAction::JoinContact { persona_hex, display_name } => {
                    let book = crate::contacts::load_local_book();
                    let target = hex::decode(&persona_hex)
                        .ok()
                        .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())
                        .and_then(|pk| crate::contacts::find(&book, &pk).cloned());
                    match target.and_then(|c| c.runtime_pubkey) {
                        Some(host_runtime) => {
                            self.begin_online_join(host_runtime, None, display_name);
                        }
                        None => {
                            self.toast = Some((
                                format!(
                                    "{display_name} hasn't played online with you yet — ask \
                                     them for an invite link."
                                ),
                                Instant::now() + std::time::Duration::from_secs(8),
                            ));
                        }
                    }
                }
                #[cfg(not(target_arch = "wasm32"))]
                crate::menu::MenuAction::JoinByInvite(link) => {
                    let now = unix_now();
                    match crate::invite::Invite::parse(&link, now) {
                        Ok(inv) => {
                            let name = if inv.world_name.is_empty() {
                                "Your friend".to_string()
                            } else {
                                inv.world_name.clone()
                            };
                            self.begin_online_join(
                                inv.host_runtime,
                                Some(inv.bearer),
                                name,
                            );
                        }
                        Err(e) => {
                            self.toast = Some((
                                e.to_string(),
                                Instant::now() + std::time::Duration::from_secs(8),
                            ));
                        }
                    }
                }
                #[cfg(not(target_arch = "wasm32"))]
                crate::menu::MenuAction::AddFriendNpub(npub) => {
                    let now = unix_now();
                    match nostr::PublicKey::parse(&npub) {
                        Ok(pk) => {
                            let mut book = crate::contacts::load_local_book();
                            crate::contacts::upsert(
                                &mut book,
                                crate::contacts::Contact {
                                    pubkey: pk.to_bytes(),
                                    display_name: None,
                                    tier: crate::comms::Tier::Kith,
                                    is_child: false,
                                    runtime_pubkey: None,
                                    added_via: crate::contacts::AddedVia::Paste,
                                    added_at: now,
                                },
                            );
                            let _ = crate::contacts::save_mirror(
                                &crate::contacts::mirror_path(),
                                &book,
                            );
                            menu_state.contacts = book;
                            self.toast = Some((
                                "Friend added.".to_string(),
                                Instant::now() + std::time::Duration::from_secs(4),
                            ));
                        }
                        Err(_) => {
                            self.toast = Some((
                                "That address didn't look right.".to_string(),
                                Instant::now() + std::time::Duration::from_secs(6),
                            ));
                        }
                    }
                }
```

Add the two small helpers on `GameState`:

```rust
    /// Bind, gather, publish an offer, and park the join for `poll` to drive.
    #[cfg(not(target_arch = "wasm32"))]
    fn begin_online_join(
        &mut self,
        host_runtime: [u8; 32],
        bearer: Option<[u8; 16]>,
        display_name: String,
    ) {
        let settings = crate::graphics_settings::GraphicsSettings::load();
        let now = unix_now();
        let started = (|| -> Result<crate::online_join::OnlineJoin, String> {
            let identity = crate::runtime_identity::RuntimeIdentity::load()?
                .ok_or_else(|| "no runtime identity".to_string())?;
            let persona = crate::signet::native_signer::current_owner_pubkey()
                .and_then(|h| hex::decode(h).ok())
                .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())
                .ok_or_else(|| {
                    "Sign in with your Signet persona to play online.".to_string()
                })?;
            // The joiner binds an ephemeral port: nothing needs to dial it by a
            // known number, and it avoids clashing with a host on this machine.
            let (socket, _punch, candidates, mapping) = bind_and_gather(0)?;
            self.upnp_mapping = mapping;
            let relay = crate::rendezvous::relay_client::MultiRelay::start(
                settings.online_relays.clone(),
            );
            crate::online_join::OnlineJoin::start(
                identity,
                persona,
                Box::new(relay),
                socket,
                candidates,
                host_runtime,
                bearer,
                display_name.clone(),
                now,
            )
        })();
        match started {
            Ok(j) => self.online_join = Some((j, Instant::now())),
            Err(e) => {
                self.toast = Some((e, Instant::now() + std::time::Duration::from_secs(8)))
            }
        }
    }

    /// Drive the online host and any in-flight join. Called once per frame from
    /// the same place `hosted_server.tick()` is driven. Never blocks.
    #[cfg(not(target_arch = "wasm32"))]
    fn poll_online(&mut self) {
        let now = unix_now();

        if let (Some(host), Some(server)) =
            (self.online_host.as_mut(), self.hosted_server.as_mut())
        {
            let players = server.server.players.len();
            for event in host.poll(now, players, players.max(5)) {
                match event {
                    crate::online_host::HostEvent::AllowlistChanged(list) => {
                        server.set_access_policy(true, list, Vec::new());
                    }
                    crate::online_host::HostEvent::ContactAdded(c) => {
                        log::info!("[online] new contact admitted by invite: {:?}", c.tier);
                    }
                    crate::online_host::HostEvent::Admitted { .. } => {}
                    crate::online_host::HostEvent::Dropped(why) => {
                        log::debug!("[online] dropped an offer: {why}")
                    }
                }
            }
        }
        if let Some(m) = self.upnp_mapping.as_mut()
            && let Err(e) = m.renew(Instant::now())
        {
            log::warn!("[online] could not renew the router mapping: {e}");
        }

        let Some((join, started)) = self.online_join.as_mut() else {
            return;
        };
        match join.poll(now, started.elapsed()) {
            crate::online_join::JoinStep::Waiting
            | crate::online_join::JoinStep::Connecting { .. } => {}
            crate::online_join::JoinStep::Ready { world_name, .. } => {
                if let Some(contact) = join.host_contact() {
                    let mut book = crate::contacts::load_local_book();
                    crate::contacts::upsert(&mut book, contact);
                    let _ =
                        crate::contacts::save_mirror(&crate::contacts::mirror_path(), &book);
                }
                if let Some(transport) = join.take_transport() {
                    match crate::signet::native_signer::restore_signer() {
                        Ok(Some(session)) => {
                            let client =
                                crate::remote_client::RemoteClient::connect_authed_on_transport(
                                    transport,
                                    "Player",
                                    native_join_sign_driver(session),
                                    None,
                                );
                            self.remote_client = Some(client);
                            self.toast = Some((
                                format!("Joined {world_name}."),
                                Instant::now() + std::time::Duration::from_secs(5),
                            ));
                        }
                        _ => {
                            self.toast = Some((
                                "Sign in with your Signet persona to play online.".to_string(),
                                Instant::now() + std::time::Duration::from_secs(8),
                            ));
                        }
                    }
                }
                self.online_join = None;
            }
            crate::online_join::JoinStep::Failed(message) => {
                self.toast =
                    Some((message, Instant::now() + std::time::Duration::from_secs(10)));
                self.online_join = None;
            }
        }
    }
```

Call `self.poll_online();` (native-gated) immediately after the existing
`hosted_server.tick()` call in the 20 TPS accumulator, and handle the two new
`CommandResult` variants beside `CommandResult::RoomInvite`:

Use the existing `CommandResult::RoomStatus` arm as the template — copy its
chat-log field and `ChatLine` constructors verbatim rather than the names
written here, which are illustrative:

```rust
            #[cfg(not(target_arch = "wasm32"))]
            CommandResult::OnlineStatus => match self.online_host.as_ref() {
                Some(h) => {
                    let (up, total) = h.relays_connected();
                    self.chat_log.push(crate::chat_ui::ChatLine::info(
                        crate::friends_ui::reachability_summary(h.candidates()),
                        tick,
                    ));
                    self.chat_log.push(crate::chat_ui::ChatLine::info(
                        format!("Relays connected: {up}/{total}"),
                        tick,
                    ));
                    self.chat_log
                        .push(crate::chat_ui::ChatLine::info(h.invite_link(), tick));
                    self.show_online_panel = true;
                }
                None => self.chat_log.push(crate::chat_ui::ChatLine::error(
                    "This world isn't being hosted online. Open it from the lobby with \
                     'Host online'.",
                    tick,
                )),
            },
            #[cfg(not(target_arch = "wasm32"))]
            CommandResult::OnlineCopyInvite => match self.online_host.as_ref() {
                Some(h) => {
                    self.pending_clipboard = Some(h.invite_link());
                    self.chat_log.push(crate::chat_ui::ChatLine::success(
                        "Invite link copied.",
                        tick,
                    ));
                }
                None => self.chat_log.push(crate::chat_ui::ChatLine::error(
                    "This world isn't being hosted online.",
                    tick,
                )),
            },
```

If `GameState` has no `pending_clipboard: Option<String>` field, add one
(native-gated) and drain it once per frame with `ctx.copy_text(text)` where the
egui context is available — clipboard writes need the egui context, which the
dispatch site does not have.

Finally, release the router mapping when hosting stops: wherever
`self.hosted_server = None;` is assigned on leaving a world, add

```rust
                #[cfg(not(target_arch = "wasm32"))]
                {
                    self.online_host = None;
                    if let Some(m) = self.upnp_mapping.take() {
                        m.remove();
                    }
                }
```

- [ ] **Step 7: Run the whole suite and the gates**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo test --bin axenstax-engine 2>&1 | tail -20
```
Expected: PASS. Then:
```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo clippy -- -D warnings 2>&1 | tail -20
```
Expected: no warnings (every `CommandResult` match is exhaustive again). Then:
```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 trunk build 2>&1 | tail -10
```
Expected: `success`.

- [ ] **Step 8: Commit**

```bash
git add game/engine/src/game_loop.rs game/engine/src/commands && git commit -m "$(cat <<'EOF'
feat(online): game-loop dispatch + /online (P4)

Host online binds, asks the router, gathers candidates, starts HostedServer on
that socket with contacts-at-Kin/Kith seeded into the allowlist, and mints the
invite. poll_online drives the host and any in-flight join each tick without
blocking; the winning transport goes through the SAME authed handshake
native_join_sign_driver already drives. The router mapping is renewed hourly and
released when hosting stops.

Claude-Session: https://claude.ai/code/session_011gW6q5QFBTbF1BNMHiayce
EOF
)" -- game/engine/src/game_loop.rs game/engine/src/commands
```

---

### Task 19: In-process integration test (`test_integration/online_play.rs`)

**Files:**
- Create: `game/engine/src/test_integration/online_play.rs`
- Modify: `game/engine/src/test_integration/mod.rs`

**Interfaces:**
- Consumes everything from Tasks 1–18. No new production code.

- [ ] **Step 1: Write the tests**

Create `game/engine/src/test_integration/online_play.rs`:

```rust
//! Two players, two loopback sockets, one in-memory relay: the whole rendezvous
//! end to end with no network policy involved.
//!
//! What this covers that the per-module tests do not: that the host's answer is
//! actually openable by the joiner it was addressed to, that the candidates the
//! joiner receives are the ones the host gathered on its real socket, and that
//! the refusal paths behave the same way when both halves are wired together
//! rather than driven by hand.
//!
//! The QUIC half is deliberately NOT driven here — the connect race is covered
//! by `nat::punch`'s pure state machine and by
//! `handshake::connect_to_server_on_socket_*`. What is left is the two-machine
//! live test on the owner's test sheet.

use nostr::Keys;

use crate::comms::Tier;
use crate::contacts::{AddedVia, Contact};
use crate::online_host::{HostEvent, OnlineHost};
use crate::online_join::{JoinStep, OnlineJoin};
use crate::rendezvous::payload::{npub_of, Candidate};
use crate::rendezvous::relay_client::FakeRelayHub;
use crate::runtime_identity::{mint_player_attestation, RuntimeIdentity};

const NOW: u64 = 1_700_000_000;

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

struct Player {
    persona: Keys,
    identity: RuntimeIdentity,
}

impl Player {
    fn new() -> Self {
        let persona = Keys::generate();
        let runtime = Keys::generate();
        let attestation = rt()
            .block_on(mint_player_attestation(
                &persona,
                &runtime.public_key(),
                nostr::Timestamp::from(NOW - 10),
                90,
            ))
            .unwrap();
        Player {
            persona,
            identity: RuntimeIdentity::from_parts(runtime, Some(attestation)),
        }
    }
    fn persona_bytes(&self) -> [u8; 32] {
        self.persona.public_key().to_bytes()
    }
    fn runtime_bytes(&self) -> [u8; 32] {
        self.identity.runtime_pubkey().to_bytes()
    }
}

fn lan(addr: &str) -> Candidate {
    Candidate {
        kind: "lan".to_string(),
        addr: addr.to_string(),
    }
}

fn host_for(hub: &FakeRelayHub, host: Player, book: Vec<Contact>) -> (OnlineHost, Player) {
    let persona = host.persona_bytes();
    let sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let identity = RuntimeIdentity::from_parts(
        host.identity.keys().clone(),
        host.identity.attestation().cloned(),
    );
    let h = OnlineHost::start(
        identity,
        persona,
        Box::new(hub.client()),
        sock,
        vec![lan("127.0.0.1:7700")],
        "Ivy's Hollow".to_string(),
        vec!["wss://nos.lol".to_string()],
        book,
        NOW,
    )
    .unwrap();
    (h, host)
}

fn join_for(hub: &FakeRelayHub, joiner: &Player, host_runtime: [u8; 32], bearer: Option<[u8; 16]>)
    -> OnlineJoin
{
    let identity = RuntimeIdentity::from_parts(
        joiner.identity.keys().clone(),
        joiner.identity.attestation().cloned(),
    );
    let sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    OnlineJoin::start(
        identity,
        joiner.persona_bytes(),
        Box::new(hub.client()),
        sock,
        vec![lan("127.0.0.1:0")],
        host_runtime,
        bearer,
        "Rowan".to_string(),
        NOW,
    )
    .unwrap()
}

#[test]
fn an_invite_admits_a_stranger_and_both_sides_end_up_as_contacts() {
    let hub = FakeRelayHub::new();
    let joiner = Player::new();
    let (mut host, host_player) = host_for(&hub, Player::new(), vec![]);
    let bearer = host.invite().bearer;

    let mut join = join_for(&hub, &joiner, host_player.runtime_bytes(), Some(bearer));

    // The host sees the offer, admits it, and answers.
    let events = host.poll(NOW, 1, 5);
    assert!(
        events.iter().any(|e| matches!(e, HostEvent::ContactAdded(_))),
        "the host records the caller: {events:?}"
    );
    assert!(host.allowlist().contains(&joiner.persona_bytes()));

    // The joiner sees the answer and moves on to connecting.
    let step = join.poll(NOW, std::time::Duration::from_millis(50));
    assert_eq!(
        step,
        JoinStep::Connecting { world_name: "Ivy's Hollow".to_string() },
        "an accepted answer must name the world"
    );
    let host_contact = join.host_contact().expect("the host becomes the joiner's contact too");
    assert_eq!(host_contact.pubkey, host_player.persona_bytes());
    assert_eq!(host_contact.tier, Tier::Kith);
    assert_eq!(host_contact.added_via, AddedVia::Invite);
}

#[test]
fn a_known_contact_joins_with_no_bearer_at_all() {
    let hub = FakeRelayHub::new();
    let joiner = Player::new();
    let book = vec![Contact {
        pubkey: joiner.persona_bytes(),
        display_name: Some("Rowan".to_string()),
        tier: Tier::Kith,
        is_child: false,
        runtime_pubkey: Some(joiner.runtime_bytes()),
        added_via: AddedVia::Invite,
        added_at: NOW - 86_400,
    }];
    let (mut host, host_player) = host_for(&hub, Player::new(), book);
    let mut join = join_for(&hub, &joiner, host_player.runtime_bytes(), None);

    host.poll(NOW, 1, 5);
    assert!(matches!(
        join.poll(NOW, std::time::Duration::from_millis(50)),
        JoinStep::Connecting { .. }
    ));
}

#[test]
fn a_stranger_is_met_with_silence_and_then_the_didnt_answer_copy() {
    let hub = FakeRelayHub::new();
    let joiner = Player::new();
    let (mut host, host_player) = host_for(&hub, Player::new(), vec![]);
    // No bearer, not a contact.
    let mut join = join_for(&hub, &joiner, host_player.runtime_bytes(), None);

    host.poll(NOW, 1, 5);
    assert!(host.allowlist().is_empty());
    assert_eq!(
        join.poll(NOW, std::time::Duration::from_millis(50)),
        JoinStep::Waiting,
        "nothing came back"
    );
    assert_eq!(
        join.poll(NOW, crate::online_join::ANSWER_TIMEOUT),
        JoinStep::Failed(crate::online_join::failure_copy(
            crate::online_join::JoinFailure::NoAnswer,
            "Rowan"
        )),
        "silence eventually reads as 'they didn't answer'"
    );
}

#[test]
fn an_expired_bearer_is_also_met_with_silence() {
    let hub = FakeRelayHub::new();
    let joiner = Player::new();
    let (mut host, host_player) = host_for(&hub, Player::new(), vec![]);
    let bearer = host.invite().bearer;
    let expiry = host.invite().expires_at;
    let mut join = join_for(&hub, &joiner, host_player.runtime_bytes(), Some(bearer));

    // Poll the host as if the invite had already lapsed.
    host.poll(expiry + 1, 1, 5);
    assert!(host.allowlist().is_empty());
    assert_eq!(
        join.poll(expiry + 1, std::time::Duration::from_millis(50)),
        JoinStep::Waiting,
        "a lapsed bearer must not be told it lapsed — that confirms a host is here"
    );
}

#[test]
fn a_full_world_is_explained_end_to_end() {
    let hub = FakeRelayHub::new();
    let joiner = Player::new();
    let book = vec![Contact {
        pubkey: joiner.persona_bytes(),
        display_name: Some("Rowan".to_string()),
        tier: Tier::Kin,
        is_child: false,
        runtime_pubkey: Some(joiner.runtime_bytes()),
        added_via: AddedVia::Kenspeckle,
        added_at: 0,
    }];
    let (mut host, host_player) = host_for(&hub, Player::new(), book);
    let mut join = join_for(&hub, &joiner, host_player.runtime_bytes(), None);

    host.poll(NOW, 5, 5);
    assert_eq!(
        join.poll(NOW, std::time::Duration::from_millis(50)),
        JoinStep::Failed(crate::online_join::failure_copy(
            crate::online_join::JoinFailure::Full,
            "Rowan"
        ))
    );
}

#[test]
fn nothing_the_relay_saw_names_a_person_a_world_or_an_address() {
    // The red-line-3 assertion at the level that matters: everything that
    // crossed the fake relay during a real admission.
    let hub = FakeRelayHub::new();
    let joiner = Player::new();
    let (mut host, host_player) = host_for(&hub, Player::new(), vec![]);
    let bearer = host.invite().bearer;
    let observer = hub.client();
    // An observer subscribed to BOTH kinds, addressed to both parties.
    observer
        .subscribe(
            crate::rendezvous::payload::KIND_JOIN_OFFER,
            &hex::encode(host_player.runtime_bytes()),
        )
        .unwrap();
    observer
        .subscribe(
            crate::rendezvous::payload::KIND_JOIN_ANSWER,
            &hex::encode(joiner.runtime_bytes()),
        )
        .unwrap();

    let mut join = join_for(&hub, &joiner, host_player.runtime_bytes(), Some(bearer));
    host.poll(NOW, 1, 5);
    let _ = join.poll(NOW, std::time::Duration::from_millis(50));

    let mut seen = 0;
    while let Some(ev) = observer.try_recv() {
        seen += 1;
        let wire = serde_json::to_string(&ev).unwrap();
        assert!(!wire.contains("Ivy"), "world name leaked: {wire}");
        assert!(!wire.contains("127.0.0.1"), "address leaked: {wire}");
        assert!(
            !wire.contains(&npub_of(&host_player.persona.public_key())),
            "host persona leaked"
        );
        assert!(
            !wire.contains(&npub_of(&joiner.persona.public_key())),
            "joiner persona leaked"
        );
        assert!(!wire.contains(&hex::encode(bearer)), "bearer leaked");
    }
    assert_eq!(seen, 2, "an offer and an answer should both have crossed the relay");
}
```

- [ ] **Step 2: Register the module**

In `game/engine/src/test_integration/mod.rs`:

```rust
// Online play by contact — the rendezvous end to end over an in-memory relay.
#[cfg(not(target_arch = "wasm32"))]
mod online_play;
```

- [ ] **Step 3: Run them to verify they pass**

```bash
cd game/engine && CARGO_BUILD_JOBS=4 nice -n 10 cargo test --bin axenstax-engine online_play 2>&1 | tail -20
```
Expected: PASS — 6 passed.

- [ ] **Step 4: Commit**

```bash
git add game/engine/src/test_integration && git commit -m "$(cat <<'EOF'
test(online): in-process host↔joiner rendezvous over an in-memory relay (P4)

Six paths wired together: invite admits a stranger and makes both sides
contacts; a known contact joins with no bearer; a stranger and an expired bearer
both get silence that resolves into the "didn't answer" copy; a full world is
explained; and an observer subscribed to both kinds sees no world name, no
address, no persona, no bearer.

Claude-Session: https://claude.ai/code/session_011gW6q5QFBTbF1BNMHiayce
EOF
)" -- game/engine/src/test_integration
```

---

### Task 20: Docs, spec maintenance, and the test sheet

**Files:**
- Create: `docs/player-guide/play-with-a-friend-online.md`
- Create: `docs/test-sheets/2026-09-06-online-play.md`
- Modify: `docs/player-guide/index.md`
- Modify: `tools/sites/wiki/app.py`
- Modify: `docs/spec/04-networking.md` (new §1.9, after §1.8.6)
- Modify: `docs/operators/host-overview.md`
- Modify: `docs/foundations/README.md` (mark the spec built)
- Modify: `CLAUDE.md` (Key Patterns — one paragraph)

**Interfaces:** none — documentation only. The spec is the source of truth
(CLAUDE.md, "Spec Maintenance"), so this task is not optional polish.

- [ ] **Step 1: Write the player-guide page**

Create `docs/player-guide/play-with-a-friend-online.md`:

```markdown
# Play with a friend online

You can host a world on your own computer and let a friend in another house
join it. Your computer is the server. Nothing runs on ours.

## What you both need

- The **desktop app** (this doesn't work in the browser).
- To be **signed in** with your Signet persona, on both computers.
- To be **online at the same time** — you're calling each other, so you both
  have to pick up.

## Hosting

1. In the lobby, find your world and press **Host online**.
2. The game copies an **invite link** to your clipboard. Send it to your friend
   however you normally talk to them.
3. That's it — you're hosting. Type `/online` in the game any time to see the
   link again, or `/online copy` to copy it.

The invite lasts **two days**. Press **Host online** again for a fresh one; the
old link stops working straight away.

### "Friends outside your home probably can't reach you"

If you see this, your router isn't letting people in. Turn on **UPnP** in your
router settings and host again. (It's usually under "Advanced", "NAT", or
"Port forwarding".) Your friends on the same wifi can still join either way.

## Joining

**The first time**, use the invite link:

1. Lobby → **Friends & servers** → **Add a friend**.
2. Paste the link, press **Add**. The game calls them straight away.

**After that** they're in your friends list. Press **Join** next to their name
whenever they're hosting — no link needed.

## Your address

At the top of the Friends column is **your address** — an `npub`, with a QR code
and a **Copy** button. Give it to a friend so they can invite you. It's just an
address; nobody can do anything with it except invite you.

## If it doesn't work

| What you see | What to do |
|---|---|
| *"… didn't answer. Are they online with the world open?"* | They need the game open with the world hosted. Ask them. |
| *"Couldn't reach …'s world."* | Their router needs UPnP turned on. Ask them to check Settings → Online. |
| *"You're on different versions."* | One of you needs to update the game. |
| *"…'s world is full."* | Wait for a space. |

## Who can join

Only people in your friends list at **Kin** or **Kith**, plus anyone holding a
live invite link. Everyone else gets nothing at all — the game doesn't even
answer them, so a stranger can't tell whether you're there.

## What the relays see

Setting up a connection uses public Nostr relays for a few messages. They carry
**only** the setup handshake, and it's encrypted: a relay sees two temporary
keys and a timestamp, and never sees who you are, what world you're playing, or
where either of you is. Once you're connected, the game goes **straight** between
your two computers. You can change which relays are used in Settings → Online.
```

- [ ] **Step 2: Link it from the guide index and the wiki**

In `docs/player-guide/index.md`, add to the Sections list next to the
multiplayer entry:

```markdown
- **[Play with a friend online](play-with-a-friend-online.md)** — host a world at home and let a friend in another house join by invite
```

In `tools/sites/wiki/app.py`, add a row to the `player-guide` page list, next to
the multiplayer entry:

```python
            {"path": "player-guide/play-with-a-friend-online.md", "title": "Play with a Friend Online", "short": "Host a world at home; a friend in another house joins by invite link"},
```

- [ ] **Step 3: Add Spec 04 §1.9**

In `docs/spec/04-networking.md`, immediately after §1.8.6 and before the `---`
that precedes "## 2. Packet Format", insert:

```markdown
### 1.9 Online Play by Contact (peer-to-peer NAT traversal)

**Implemented 2026-09-06.** Design:
`docs/superpowers/specs/2026-09-06-online-play-by-contact-design.md`. This
supersedes §1.7's "platform-operated STUN server" and "TURN-like relay operated
by the platform" for personal-tier hosts: **AxeNStax operates neither.** STUN is
two public servers; there is no forwarder in this version, and when one exists it
is operator-run software, never ours (CLAUDE.md red line 2).

**Identity.** A player's address is their Signet persona npub. Signalling is
signed by a per-install **runtime key** the persona attests once — a kind-30420
event (`server_identity::attestation`) carrying the tag `role=player`; `role`
absent still means a server delegation. The attestation travels inside the
encrypted payload and is never published, so a relay cannot correlate a runtime
key to a person.

**Signalling.** Two ephemeral kinds, both NIP-44 sealed runtime-key to
runtime-key and `p`-tagged to the recipient:

| Kind | Name | Direction | Content |
|---|---|---|---|
| 20900 | `join-offer` | joiner → host | `Offer { v, session, persona, attestation, bearer?, protocol, candidates, sent_at }` |
| 20901 | `join-answer` | host → joiner | `Answer { v, session, persona, attestation, accepted, reason?, protocol, candidates, world_name, sent_at }` |

Verification, in order: outer signature; kind + decrypt + parse + `v == 1`;
attestation valid, `role=player`, and naming **the outer signer**; the payload's
`persona` equal to the attestation's signer; `sent_at` within ±120 s; `session`
unseen. Any failure is a silent drop.

**Admission.** Contacts at Kin or Kith are admitted; a live 16-byte invite bearer
admits once and makes the caller a Kith contact. **Ken does not admit** — it is
hear-only in `comms.rs` and is not a key to the house. Only `ProtocolMismatch`
and `Full` are ever answered; every other refusal is silence.

**Reachability.** Candidates in priority order — `lan`, `v6`, `upnp`
(igd-next, 7200 s lease renewed hourly, released on stop), `stun` (RFC 5389
Binding Request; codec pinned to the RFC 5769 vectors) — all gathered on **one
already-bound UDP socket**, which is then handed to quinn
(`Endpoint::new(config, server_config, socket, runtime)`). Both sides send 3
`AXNS-PUNCH` datagrams per candidate 100 ms apart, then the joiner races QUIC
connects across all candidates staggered 150 ms, first ALPN completion wins,
8 s deadline.

**The QUIC join is unchanged.** `ChallengePacket` → `JoinRequest` carrying the
persona-signed kind-21236 `auth_event` → `access_policy`. `PROTOCOL_VERSION` is
untouched; the `protocol` field in the offer exists only so a mismatch is
explained before a connect is attempted. `HostedServer.require_signin` stays
`true`, and the online host's allowlist is contacts at Kin/Kith plus this
session's bearer admissions.

**Not built here (deliberate):** relay/forwarder fallback, presence, web, voice,
NAT-PMP, Signet contacts import (waits upstream), multiple simultaneous hosted
worlds.
```

- [ ] **Step 4: Add the operator note**

In `docs/operators/host-overview.md`, under "Why run your own?", add:

```markdown
> **Just playing with a friend?** You don't need any of this. The desktop app
> can host a world straight from your own machine and let a friend in another
> house join by invite link — see
> [Play with a friend online](../player-guide/play-with-a-friend-online.md).
> Run a dedicated server when you want a world that stays up whether or not
> you're playing.
```

- [ ] **Step 5: Write the test sheet**

Create `docs/test-sheets/2026-09-06-online-play.md`:

```markdown
# Test sheet — Online play by contact (2026-09-06)

**Build:** v0.2.25
**Needs:** two machines, two Signet personas, two different networks (laptop A
on home wifi, laptop B on a phone hotspot). Signer = any NIP-46 bunker via the
paste path (Amber / nsec.app) — mySignet cannot be the native signer until
upstream Signet ticket 187 lands.

## Setup

- [ ] A: sign in with persona A. First "Host online" asks the phone to approve
      this device — approve it.
- [ ] B: sign in with persona B, on a **different network** (phone hotspot).
- [ ] Both: Settings → Online shows a port and four relays.

## 1. Host and invite

- [ ] A: lobby → world card → **Host online**. The world opens.
- [ ] A: a toast says the invite link was copied (or warns that friends outside
      the home probably can't reach you — write down which).
- [ ] A: `/online` prints a reachability line, a relay count, and the link.
      **Reachability said:** ____________________
      **Relays connected:** ____ / ____
- [ ] Send the link to B.

## 2. Join by invite

- [ ] B: lobby → Friends & servers → **Add a friend** → paste the link → Add.
- [ ] B lands in A's world within about ten seconds.
      **How long did it take?** ______
- [ ] Both can see each other move and place blocks.
- [ ] A: the player-inspect view shows B's full npub.

## 3. Join again as a contact

- [ ] B: leave, return to the lobby. A's name is now in the **Friends** list.
- [ ] B: press **Join** next to it — no link, no pasting.
- [ ] B lands in A's world again.

## 4. Nobody home

- [ ] A: leave the world (stop hosting).
- [ ] B: press **Join** again.
- [ ] After about eight seconds B sees:
      *"<Name> didn't answer. Are they online with the world open?"*
      **Exact wording seen:** ____________________

## 5. A stranger gets nothing

- [ ] A: host online again, then press **Host online** again to mint a **fresh**
      invite (this retires the old bearer).
- [ ] B: paste the **old** link.
- [ ] B waits the full eight seconds and gets the "didn't answer" message — NOT
      "your invite expired". (The host must not confirm it is there.)

## 6. Router behaviour

- [ ] A: check the router's admin page — there is a UDP mapping described
      "AxeNStax" while hosting.
- [ ] A: stop hosting. Within a moment the mapping is gone.

## Questions for Axolittle

1. Did the invite link feel like something you'd actually send someone?
2. When it failed, did the message tell you what to do next?
3. Was "Friends & servers" the place you looked for a friend?
4. Anything that felt slow?

## Notes
```

- [ ] **Step 6: Update the foundations queue and CLAUDE.md**

In `docs/foundations/README.md`, add the design doc to the built list with the
date and a one-line summary.

In `CLAUDE.md`, under **Key Patterns**, add:

```markdown
### Online play by contact (native)

A host binds one UDP socket, gathers reachability candidates on it (LAN, IPv6,
UPnP via `igd-next`, STUN), and hands that socket to quinn. A joiner picks a
**contact** — a Signet persona npub — and the two exchange an encrypted
offer/answer over public Nostr relays (kinds 20900/20901, NIP-44, signed by a
per-install **runtime key** the persona attested once with `role=player`), punch
through their routers, and connect **directly**. The QUIC join handshake and
`PROTOCOL_VERSION` are unchanged. Admission is "in my contacts at Kin or Kith,
or holding my live invite bearer"; everyone else gets silence. Relays carry
setup only and never game traffic. Modules: `invite.rs`, `runtime_identity.rs`,
`online_admission.rs`, `rendezvous/`, `nat/`, `online_host.rs`,
`online_join.rs`, `friends_ui.rs`. Spec:
`docs/superpowers/specs/2026-09-06-online-play-by-contact-design.md`, Spec 04
§1.9. Native only — the web build carries none of it, and
`tools/smoke/forbidden-symbol.mjs` proves that.
```

- [ ] **Step 7: Verify the wiki renders the new page**

```bash
tools/sites/wiki/start.sh
```
Then:
```bash
curl -sk https://localhost:8097/docs/player-guide/play-with-a-friend-online.md | head -20
```
Expected: the page's rendered heading, not a 404. Stop it afterwards with
`tools/sites/stop-all.sh`.

- [ ] **Step 8: Commit**

```bash
git add docs/player-guide/play-with-a-friend-online.md docs/player-guide/index.md docs/test-sheets/2026-09-06-online-play.md docs/spec/04-networking.md docs/operators/host-overview.md docs/foundations/README.md tools/sites/wiki/app.py CLAUDE.md && git commit -m "$(cat <<'EOF'
docs(online): player guide, Spec 04 §1.9, operator note, test sheet (P4)

§1.9 supersedes §1.7's platform-operated STUN and TURN for personal hosts: we
operate neither. The player page explains what the relays do and do not see, in
words a kid can read. Test sheet covers the two-machine live path, including the
one that must NOT happen — a retired bearer being told it expired.

Claude-Session: https://claude.ai/code/session_011gW6q5QFBTbF1BNMHiayce
EOF
)" -- docs/player-guide/play-with-a-friend-online.md docs/player-guide/index.md docs/test-sheets/2026-09-06-online-play.md docs/spec/04-networking.md docs/operators/host-overview.md docs/foundations/README.md tools/sites/wiki/app.py CLAUDE.md
```

---

## Phase 5 — Ship

### Task 21: check.sh green, version bump, AppImage, push

**Files:**
- Modify: `game/engine/Cargo.toml` (`version = "0.2.25"`)
- Modify: `game/engine/Cargo.lock` (the `axenstax-engine` entry, line ~384)
- Modify: `tools/packaging/packager.toml` (`version = "0.2.25"`, line 21)

**Interfaces:** none — release mechanics.

- [ ] **Step 1: Run the full gate in the FOREGROUND**

```bash
./check.sh 2>&1 | tail -40
```
Expected: `ALL GREEN`, and the closing "what this run verified" list naming
version parity, docs-site unit tests, clippy `-D warnings`, native build, cargo
test, trunk wasm build, forbidden-symbol gate, and bundle size. Playwright smoke
is listed as NOT RUN — that is expected without `--smoke`.

**If the forbidden-symbol gate fails**, a native-only symbol reached the wasm
bundle: find the ungated free function (a free fn is NOT covered by its caller's
`#[cfg]`) and gate it. Do not weaken the gate.

- [ ] **Step 2: Bump the version in all three places**

`game/engine/Cargo.toml`:
```toml
version = "0.2.25"
```

`game/engine/Cargo.lock` — the `[[package]] name = "axenstax-engine"` entry:
```toml
version = "0.2.25"
```

`tools/packaging/packager.toml`:
```toml
version = "0.2.25"                       # keep in step with game/engine/Cargo.toml
```

- [ ] **Step 3: Re-run the gate to prove version parity**

```bash
./check.sh 2>&1 | tail -40
```
Expected: `ALL GREEN`, with `OK engine and packager versions agree (0.2.25)` in
the first section. This gate exists because v0.2.16 shipped with the two out of
step and the download page advertised the wrong version for four days.

- [ ] **Step 4: Commit the bump**

```bash
git add game/engine/Cargo.toml game/engine/Cargo.lock tools/packaging/packager.toml && git commit -m "$(cat <<'EOF'
chore(release): v0.2.25 — online play by contact

Claude-Session: https://claude.ai/code/session_011gW6q5QFBTbF1BNMHiayce
EOF
)" -- game/engine/Cargo.toml game/engine/Cargo.lock tools/packaging/packager.toml
```

- [ ] **Step 5: Build the AppImage and put it on the Desktop**

```bash
nice -n 10 tools/packaging/build-local.sh appimage 2>&1 | tail -20
```
Expected: an `*_x86_64.AppImage` (and its `.zsync`) under
`tools/packaging/staging/`. Then:
```bash
cp "$(find tools/packaging/staging -maxdepth 2 -name '*_x86_64.AppImage' | head -1)" ~/Desktop/ && ls -lh ~/Desktop/*.AppImage
```
Expected: the AppImage on the Desktop, ~100 MB.

Note: `build-local.sh` sets `TMPDIR` into the build tree because `/tmp` is
mounted `noexec` on this host — do not override it.

- [ ] **Step 6: Confirm the built binary reports the new version**

```bash
~/Desktop/*.AppImage --version 2>&1 | head -3
```
Expected: `0.2.25`. (If the binary has no `--version` flag, launch it and read
the version badge in the lobby corner instead.)

- [ ] **Step 7: Push**

```bash
git log --oneline -22 && git push origin main
```
Expected: 20 feature/docs commits (Tasks 1–20) plus the release commit, then a
clean push.
The push to `main` triggers the web deploy automatically (GitHub Actions →
Hetzner); nothing else is needed for the web side.

**Do not** trigger `native-packages.yml` or `publish-installers.yml` in this
task — a linux-only native release is pre-authorised, but the owner has a
locally built AppImage on the Desktop to test first. Ask before spending CI
minutes on mac/Windows.

- [ ] **Step 8: Hand over the live test**

The remaining gap is the owner boundary and cannot be closed solo: two machines,
two personas, two networks. Point the owner at
`docs/test-sheets/2026-09-06-online-play.md` and note that mySignet still cannot
be the native signer (upstream Signet ticket 187), so the paste path with any
NIP-46 bunker is the way in.

---

## Self-review

Run after the plan is written; findings fixed inline.

**1. Spec coverage.**

| Spec section | Task(s) |
|---|---|
| §1 red lines | Global Constraints; Task 1 step 6 (forbidden-symbol); Task 6 leak test; Task 19 observer test |
| §2 Persona / runtime key / attestation | Task 2 |
| §2 Contact (extended fields, mirror, union) | Task 3 |
| §2 Invite | Task 1 |
| §3.1 Invite link + QR | Task 1; Task 17 (QR in the Online panel) |
| §3.2 Signalling events + verification | Tasks 6, 7 |
| §3.3 Admission rule + silence + mutual contact | Tasks 4, 15, 16 |
| §3.4 QUIC join unchanged | Task 14 (`connect_authed_on_transport`), Task 18 (sign driver) |
| §4.1 Candidates (lan/v6/upnp/stun) | Tasks 10, 11, 12 |
| §4.2 Socket handoff | Task 14 |
| §4.3 Punch + parallel connect race | Tasks 13, 14 |
| §4.4 Failure copy (4 strings) + host warning | Task 16 (verbatim, pinned); Task 15 (`unreachable_warning`) |
| §5.1 Host flow (1–6) | Tasks 15, 17, 18 |
| §5.2 Joiner flow (1–3) | Tasks 16, 17, 18 |
| §5.3 Your address | Task 5 |
| §6 Storage + settings | Tasks 2, 3, 9 |
| §7 Module table | File Structure; every module has a task |
| §8 Unit + integration testing | Every task's TDD steps; Task 19 |
| §8 Live test sheet | Task 20 |
| §9 Five phases | Tasks 1–5 / 6–9 / 10–14 / 15–20 / 21 |
| §10 Upstream asks | Task 20 (test sheet + spec note reference tickets 187/188) |
| §11 Out of scope | Not built; §1.9 names them explicitly |

No gaps.

**2. Placeholder scan.** No "TBD", "TODO", "implement later", "add error
handling", "similar to Task N", or test steps without test code. Task 16 Step 3
deliberately contains a wrong-looking block, and Step 4 is the fix — that is a
written-out two-step edit, not a placeholder. The `nat/{candidates,upnp,punch}.rs`
placeholder files created in Task 10 Step 4 are one line each and are fully
replaced by Tasks 11–13.

**3. Type consistency.** Checked across tasks:
`Candidate { kind: String, addr: String }` (Task 6) is what Tasks 11, 15, 16, 17
consume. `Contact`'s seven fields (Task 3) are constructed identically in Tasks
15, 16, 17, 18, 19. `RuntimeIdentity::{keys, runtime_pubkey, attestation,
persona, from_parts}` (Tasks 2, 15) match every call site.
`online_admission::{admit, admits_play, is_reply_worthy, refusal_wire,
refusal_from_wire, ActiveBearer, AdmitReason, Refusal, Admission}` (Task 4) are
used with those exact names in Tasks 15, 16, 17. `ConnectRace::{new, advance,
on_connected, on_failed, outcome}` (Task 13) match Task 14's usage.
`network::OnlineConnect { transport, outcome }` (Task 14) is destructured with
those field names in Task 16. `HostedServer::{start_online, set_access_policy,
port}` (Task 14) match Task 18. `JoinStep`/`HostEvent` variants match between
Tasks 15/16 and 18/19. `MenuAction::JoinContact { persona_hex, display_name }`
matches between Task 17 and Task 18.

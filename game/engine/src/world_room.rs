//! The world-room seam — how a world's chat reaches somebody who is not in the
//! game.
//!
//! The parent is on a phone. The child is in the world. A room is what joins
//! them, and [`WorldRoom`] is the one interface the engine knows about.
//!
//! **Wrap, don't port.** The implementation that ships is
//! [`kithmoot_keeper::KithMootKeeper`], which supervises a `kithmoot-agent`
//! child process and speaks its newline-delimited-JSON "stdio brain" seam. A
//! Rust KithMoot would be a second implementation behind this same trait, with
//! no callers changed. The trait is the concrete design; the subprocess is the
//! bridge.
//!
//! Everything in this module that can be pure, is — the relay lint, the codec,
//! the policy check — so the parts that decide anything are testable with no
//! Node, no relays and no network. Spec:
//! `docs/foundations/2026-09-05-world-chat.md` §4 and §8.2.

// BRIDGE: `KithMootKeeper` is now the production caller for this seam, so only
// the items it does not call yet (`KeeperCommand::Roster`, `is_retryable`) carry
// item-level `allow(dead_code)` — delete each when the keeper uses it.

use serde::{Deserialize, Serialize};

/// A relay that must never carry a group's conversation.
///
/// `relay.trotters.cc` is ours. It exists for discovery hints, Signet sign-in
/// and feedback. A family's chat travelling over it would make AxeNStax the
/// operator of the channel, which is the second regulatory red line
/// (`CLAUDE.md`) and the whole reason this lint exists.
///
/// This is not theoretical tidiness: KithMoot's own `DEFAULT_RELAYS` lists
/// exactly this host **first**, so a caller who passes no relays gets it by
/// default. That is why [`lint_relays`] refuses an empty list rather than
/// letting the child process fall back.
pub const FORBIDDEN_RELAY_HOST: &str = "relay.trotters.cc";

/// Why a relay list was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RelayError {
    /// No relays at all. Refused rather than defaulted — see the note on
    /// [`FORBIDDEN_RELAY_HOST`] for what the default would have been.
    Empty,
    /// Every relay given was dropped by the lint, leaving nothing usable.
    AllForbidden,
    /// Not a `wss://` (or `ws://`) URL.
    NotARelayUrl(String),
}

impl RelayError {
    /// The line an operator sees. Says what to do, not just what went wrong.
    pub fn message(&self) -> String {
        match self {
            RelayError::Empty => {
                "No relays given for the room. Supply your own — two or three \
                 wss:// URLs your family already uses. There is deliberately no \
                 default: the library's default would route your conversation \
                 through AxeNStax infrastructure."
                    .to_string()
            }
            RelayError::AllForbidden => format!(
                "Every relay given was {FORBIDDEN_RELAY_HOST}, which is AxeNStax \
                 infrastructure for discovery hints, sign-in and feedback — it \
                 must never carry a group's conversation. Supply your own relays."
            ),
            RelayError::NotARelayUrl(u) => {
                format!("Not a relay URL: {u} — relays are wss:// or ws:// addresses.")
            }
        }
    }
}

/// One relay that was dropped, and why, so the caller can say so out loud
/// rather than silently shortening the operator's list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DroppedRelay {
    pub url: String,
    pub reason: &'static str,
}

/// The result of linting an operator's relay list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LintedRelays {
    pub kept: Vec<String>,
    pub dropped: Vec<DroppedRelay>,
}

/// Check an operator-supplied relay list.
///
/// Refuses an empty list, refuses anything that is not a relay URL, and drops
/// [`FORBIDDEN_RELAY_HOST`] with a reason. The caller must pass `kept` on to the
/// child process explicitly — never let it fall back to its own defaults.
///
/// Matching is on the host, so `wss://relay.trotters.cc/` and
/// `wss://relay.trotters.cc:443` are both caught; a host that merely *contains*
/// the string (`wss://relay.trotters.cc.example.net`) is not the same host and
/// is kept, which is the correct reading — that is somebody else's relay.
pub fn lint_relays(relays: &[String]) -> Result<LintedRelays, RelayError> {
    if relays.is_empty() {
        return Err(RelayError::Empty);
    }

    let mut kept = Vec::new();
    let mut dropped = Vec::new();

    for raw in relays {
        let url = raw.trim();
        if url.is_empty() {
            continue;
        }
        let rest = match url.strip_prefix("wss://").or_else(|| url.strip_prefix("ws://")) {
            Some(r) => r,
            None => return Err(RelayError::NotARelayUrl(url.to_string())),
        };
        // Authority is everything before the first '/', '?' or '#'; the host is
        // that minus any `user@` userinfo (so `wss://x@relay.trotters.cc` can't
        // slip past), minus any ':port', minus a trailing root '.'.
        let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
        let host_port = authority.rsplit('@').next().unwrap_or("");
        let host = host_port
            .split(':')
            .next()
            .unwrap_or("")
            .trim_end_matches('.')
            .to_ascii_lowercase();

        if host == FORBIDDEN_RELAY_HOST {
            dropped.push(DroppedRelay {
                url: url.to_string(),
                reason: "AxeNStax infrastructure — never carries a group's conversation",
            });
            continue;
        }
        if host.is_empty() {
            return Err(RelayError::NotARelayUrl(url.to_string()));
        }
        kept.push(url.to_string());
    }

    if kept.is_empty() {
        return Err(if dropped.is_empty() {
            RelayError::Empty
        } else {
            RelayError::AllForbidden
        });
    }
    Ok(LintedRelays { kept, dropped })
}

// ─── The room link, and the policy we insist on ───

/// Why a room link was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkError {
    /// Not a KithMoot link at all.
    Malformed,
    /// The link does not carry `agents: "owned-by-members"`.
    ///
    /// This is a refusal, not a warning. Upstream the rule is **off by
    /// default** — "a room that says nothing admits agents as it always did" —
    /// so a link without it admits agents nobody vouches for. An unowned agent
    /// in a room with a child is an anonymous stranger with a language model
    /// attached, and the entire tier system exists so that there are no
    /// anonymous strangers.
    AgentsNotOwned,
}

impl LinkError {
    pub fn message(&self) -> &'static str {
        match self {
            LinkError::Malformed => "That does not look like a room link.",
            LinkError::AgentsNotOwned => {
                "That room does not require agents to be owned by a member, so \
                 anyone could put an unattributed bot in it. Make the room with \
                 this game, or with `--policy agents=owned-by-members`."
            }
        }
    }
}

/// The policy marker we require inside a room link's access block.
const OWNED_BY_MEMBERS: &str = "owned-by-members";

/// Check a room link before attaching to it.
///
/// A KithMoot link is `https://<host>/j/#<base64url(JSON)>`, and the access
/// block rides inside that fragment. We do not need to decode it to answer the
/// one question we care about — but we do need to be sure we are looking at the
/// fragment and not at the host, so the check is deliberately narrow.
pub fn check_room_link(link: &str) -> Result<(), LinkError> {
    let fragment = match link.split_once('#') {
        Some((_, frag)) if !frag.is_empty() => frag,
        _ => return Err(LinkError::Malformed),
    };
    // The fragment is base64url of a JSON object. Decoding it properly is the
    // implementation's job; for the policy gate it is enough that the decoded
    // form carries the marker, so callers hand us the decoded JSON when they
    // have it and the raw fragment when they do not.
    if fragment.contains(OWNED_BY_MEMBERS) {
        Ok(())
    } else {
        Err(LinkError::AgentsNotOwned)
    }
}

/// Check an already-decoded link payload for the same policy.
///
/// This is the reliable form — [`check_room_link`] can only see the marker if
/// the caller passes a decoded fragment, so the implementation decodes first
/// and calls this.
pub fn check_link_policy_json(decoded: &str) -> Result<(), LinkError> {
    if decoded.contains(OWNED_BY_MEMBERS) {
        Ok(())
    } else {
        Err(LinkError::AgentsNotOwned)
    }
}

// ─── The stdio seam ───

/// One line arriving from the room.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InboundLine {
    /// The speaker's participant pubkey, hex.
    pub from: String,
    /// Display name, if the room supplied one. Never trusted for identity.
    pub name: Option<String>,
    pub text: String,
}

/// One line going out to the room.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutboundLine {
    pub text: String,
}

/// What the child process says on stdout, as far as we care about it.
///
/// KithMoot's `StdioEvent` is wider than this; we decode the variants we act on
/// and deliberately ignore the rest rather than failing on them, so an upstream
/// addition does not break the seam.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeeperEvent {
    /// Always the first event, before anything else. Its absence within the
    /// start timeout means the start failed.
    Ready { participant: String, url: String },
    Chat(InboundLine),
    Roster { participants: Vec<RosterMember> },
    Error { message: String },
    Ok { op: String },
    /// A shape we do not act on. Kept as a variant so callers can log it rather
    /// than the codec silently swallowing it.
    Other { kind: String },
}

/// One member the room reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RosterMember {
    pub participant: String,
    pub name: Option<String>,
    pub agent: bool,
}

/// What we say on the child process's stdin.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum KeeperCommand {
    Say { text: String },
    #[allow(dead_code)] // built by the keeper bridge once KithMootKeeper asks for a roster; tested only today
    Roster,
    Leave,
}

/// The raw event shape, for serde. Kept private so the public surface stays
/// [`KeeperEvent`], which is ours and does not move when upstream's does.
#[derive(Deserialize)]
struct RawEvent {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    participant: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    op: Option<String>,
    #[serde(default)]
    participants: Option<Vec<RawRosterMember>>,
}

#[derive(Deserialize)]
struct RawRosterMember {
    participant: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    agent: bool,
}

/// Decode one line of the child process's stdout.
///
/// Returns `None` for a blank line or one that is not JSON at all — the child
/// writes human log lines to stderr, not stdout, but a stray line must not take
/// the room down.
pub fn decode_event(line: &str) -> Option<KeeperEvent> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let raw: RawEvent = serde_json::from_str(line).ok()?;
    Some(match raw.kind.as_str() {
        "ready" => KeeperEvent::Ready {
            participant: raw.participant.unwrap_or_default(),
            url: raw.url.unwrap_or_default(),
        },
        "roster" => KeeperEvent::Roster {
            participants: raw
                .participants
                .unwrap_or_default()
                .into_iter()
                .map(|m| RosterMember {
                    participant: m.participant,
                    name: m.name,
                    agent: m.agent,
                })
                .collect(),
        },
        "error" => KeeperEvent::Error {
            message: raw.message.unwrap_or_default(),
        },
        "ok" => KeeperEvent::Ok {
            op: raw.op.unwrap_or_default(),
        },
        // Every conversation channel arrives with its own `type` (chat,
        // backchannel, transcript…). Anything carrying text from somebody is a
        // line; that is the only thing this seam does with it.
        _ if raw.text.is_some() && raw.from.is_some() => KeeperEvent::Chat(InboundLine {
            from: raw.from.unwrap_or_default(),
            name: raw.name,
            text: raw.text.unwrap_or_default(),
        }),
        other => KeeperEvent::Other {
            kind: other.to_string(),
        },
    })
}

/// Encode one command as a line for the child process's stdin, newline included.
pub fn encode_command(cmd: &KeeperCommand) -> String {
    // The command shapes are small and fixed; a failure here is a bug in this
    // file rather than a runtime condition, so it is not worth a Result.
    let mut s = serde_json::to_string(cmd).unwrap_or_else(|_| "{}".to_string());
    s.push('\n');
    s
}

/// What the engine needs to attach to a room.
#[derive(Clone, Debug)]
pub struct RoomConfig {
    /// The room link, already policy-checked.
    pub link: String,
    /// What the room calls this participant.
    pub name: String,
    /// Operator-supplied relays, already linted. Passed explicitly, always.
    pub relays: Vec<String>,
}

/// Why attaching to a room failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RoomError {
    Relays(RelayError),
    Link(LinkError),
    /// The child process did not announce itself in time.
    NoReady,
    /// The room is closed. Terminal by design upstream — a closed room is never
    /// reopened, so this must not be retried.
    Closed,
    /// Anything else, with what the process said.
    Failed(String),
}

impl RoomError {
    /// Whether retrying could ever help. `Closed` is terminal upstream; retrying
    /// it forever would be a busy loop against a decision somebody made.
    #[allow(dead_code)] // the retry policy for the keeper bridge; tested only today
    pub fn is_retryable(&self) -> bool {
        match self {
            RoomError::Closed | RoomError::Relays(_) | RoomError::Link(_) => false,
            RoomError::NoReady | RoomError::Failed(_) => true,
        }
    }

    /// The line an operator sees — `/room join`'s error path prints this
    /// verbatim rather than a generic "failed" (spec §4.5). `Relays` and
    /// `Link` already carry a message written for the operator; the rest get
    /// one here.
    pub fn message(&self) -> String {
        match self {
            RoomError::Relays(e) => e.message(),
            RoomError::Link(e) => e.message().to_string(),
            RoomError::NoReady => {
                "The room didn't answer in time. Check the link and that the room process \
                 can reach its relays, then try again."
                    .to_string()
            }
            RoomError::Closed => {
                "That room is closed and won't reopen — ask whoever created it for a fresh \
                 invite."
                    .to_string()
            }
            RoomError::Failed(msg) => msg.clone(),
        }
    }
}

/// The seam. One implementation ships (`KithMootKeeper`); a test double
/// implements it to drive the mirroring tests with no Node and no network.
pub trait WorldRoom {
    /// Attach. Blocks until the room says `ready`, or the start times out.
    fn start(&mut self, cfg: &RoomConfig) -> Result<(), RoomError>;
    /// Say something in the room.
    fn post(&mut self, line: &OutboundLine) -> Result<(), RoomError>;
    /// Take whatever has arrived since the last call. Never blocks.
    fn poll(&mut self) -> Vec<InboundLine>;
    /// Who is in the room, as of the last roster we were told about.
    fn members(&self) -> Vec<RosterMember>;
    /// Leave and stop the child process.
    fn stop(&mut self) -> Result<(), RoomError>;
}

// ─── Mirroring: the room is a member of the conversation, not a bypass ───

/// Should a line spoken in the world be mirrored out to the room?
///
/// The room is a **member of the world's conversation, not a bypass of it**. A
/// line goes out only if the speaker would have been permitted to speak to at
/// least one of the room's members under the ordinary rule.
///
/// `member_tiers` is how the SPEAKER classifies each room member — the same
/// directional, local view the in-world rule uses. A member the speaker has
/// never heard of is a `Stranger`, exactly as in the world.
///
/// The consequence worth being explicit about, because it is the whole point: a
/// child on `Approved` does **not** have their lines mirrored to a room whose
/// members they have not ken'd. A family room (members are kin) sees the child's
/// chat; a crew room of the host's acquaintances does not.
///
/// Room members' own hearing is deliberately not modelled. They are adults who
/// chose to join a room, and we have no Charter ceiling for them; the speaker's
/// permission is the one that governs, and it is the one that protects a child.
pub fn should_mirror_out(speaker: crate::comms::Party, member_tiers: &[crate::comms::Tier]) -> bool {
    member_tiers
        .iter()
        .any(|tier| crate::comms::speak_ok(speaker.level, *tier))
}

/// Should a line arriving from the room reach this player?
///
/// The same hearing rule as the world: the listener's own level, narrowed by
/// their own filter, against how the LISTENER classifies the room member who
/// spoke. A room does not get to bypass a child's ceiling by virtue of being a
/// room.
pub fn should_mirror_in(
    listener: crate::comms::Party,
    listener_sees_speaker: crate::comms::Tier,
) -> bool {
    crate::comms::hear_ok(listener.hear_level(), listener_sees_speaker)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }

    // ─── relay lint ───

    #[test]
    fn empty_relay_list_is_refused_not_defaulted() {
        assert_eq!(lint_relays(&[]).unwrap_err(), RelayError::Empty);
    }

    /// The whole point: KithMoot's own default list starts with our relay, so
    /// "just pass nothing" would route a family's conversation through us.
    #[test]
    fn trotters_is_dropped_with_a_reason() {
        let got = lint_relays(&v(&["wss://relay.trotters.cc", "wss://nos.lol"])).unwrap();
        assert_eq!(got.kept, v(&["wss://nos.lol"]));
        assert_eq!(got.dropped.len(), 1);
        assert!(got.dropped[0].reason.contains("AxeNStax"));
    }

    #[test]
    fn trotters_is_caught_with_a_port_or_a_path_or_odd_case() {
        for u in [
            "wss://relay.trotters.cc/",
            "wss://relay.trotters.cc:443",
            "wss://RELAY.TROTTERS.CC",
            "ws://relay.trotters.cc",
            "wss://x@relay.trotters.cc",
            "wss://a:b@relay.trotters.cc:443/",
            "wss://relay.trotters.cc.",
            "wss://relay.trotters.cc?x=1",
        ] {
            let got = lint_relays(&v(&[u, "wss://nos.lol"])).unwrap();
            assert_eq!(got.kept, v(&["wss://nos.lol"]), "did not catch {u}");
        }
    }

    /// A different host that merely contains the string is somebody else's
    /// relay, and dropping it would be wrong.
    #[test]
    fn a_lookalike_host_is_not_our_relay() {
        let got = lint_relays(&v(&["wss://relay.trotters.cc.example.net"])).unwrap();
        assert_eq!(got.kept, v(&["wss://relay.trotters.cc.example.net"]));
        assert!(got.dropped.is_empty());
    }

    #[test]
    fn a_list_of_only_trotters_is_refused_rather_than_emptied() {
        let err = lint_relays(&v(&["wss://relay.trotters.cc"])).unwrap_err();
        assert_eq!(err, RelayError::AllForbidden);
        assert!(err.message().contains("Supply your own"));
    }

    #[test]
    fn non_relay_urls_are_refused() {
        let err = lint_relays(&v(&["https://example.com"])).unwrap_err();
        assert_eq!(
            err,
            RelayError::NotARelayUrl("https://example.com".to_string())
        );
    }

    #[test]
    fn relay_errors_tell_the_operator_what_to_do() {
        assert!(RelayError::Empty.message().contains("Supply your own"));
        assert!(RelayError::Empty.message().contains("no default"));
    }

    // ─── link policy ───

    #[test]
    fn a_link_without_owned_by_members_is_refused() {
        let err = check_link_policy_json(r#"{"v":2,"a":{"tier":"kith"}}"#).unwrap_err();
        assert_eq!(err, LinkError::AgentsNotOwned);
        assert!(err.message().contains("unattributed bot"));
    }

    #[test]
    fn a_link_with_owned_by_members_passes() {
        assert!(check_link_policy_json(r#"{"a":{"agents":"owned-by-members"}}"#).is_ok());
    }

    #[test]
    fn a_link_with_no_fragment_is_malformed() {
        assert_eq!(
            check_room_link("https://example.org/j/").unwrap_err(),
            LinkError::Malformed
        );
        assert_eq!(check_room_link("nonsense").unwrap_err(), LinkError::Malformed);
    }

    // ─── codec ───

    #[test]
    fn ready_is_decoded() {
        let line = r#"{"type":"ready","participant":"abc","device":"d","room":"r","url":"https://x/j/#z","hosting":true}"#;
        assert_eq!(
            decode_event(line),
            Some(KeeperEvent::Ready {
                participant: "abc".to_string(),
                url: "https://x/j/#z".to_string(),
            })
        );
    }

    #[test]
    fn a_chat_line_is_decoded() {
        let line = r#"{"type":"chat","id":"1","from":"abc","name":"Ada","text":"hello","sentAt":1757}"#;
        assert_eq!(
            decode_event(line),
            Some(KeeperEvent::Chat(InboundLine {
                from: "abc".to_string(),
                name: Some("Ada".to_string()),
                text: "hello".to_string(),
            }))
        );
    }

    /// Named channels arrive with their own `type`. Anything carrying text from
    /// somebody is a line — that is all this seam does with it.
    #[test]
    fn a_line_on_another_channel_is_still_a_line() {
        let line = r#"{"type":"backchannel","from":"abc","text":"psst","sentAt":1}"#;
        match decode_event(line) {
            Some(KeeperEvent::Chat(l)) => assert_eq!(l.text, "psst"),
            other => panic!("expected a chat line, got {other:?}"),
        }
    }

    #[test]
    fn roster_is_decoded() {
        let line = r#"{"type":"roster","participants":[{"participant":"a","name":"Ada","agent":false,"tracks":[]},{"participant":"b","agent":true,"tracks":[]}]}"#;
        match decode_event(line) {
            Some(KeeperEvent::Roster { participants }) => {
                assert_eq!(participants.len(), 2);
                assert_eq!(participants[0].name.as_deref(), Some("Ada"));
                assert!(!participants[0].agent);
                assert!(participants[1].agent);
                assert!(participants[1].name.is_none());
            }
            other => panic!("expected a roster, got {other:?}"),
        }
    }

    #[test]
    fn errors_and_acks_are_decoded() {
        assert_eq!(
            decode_event(r#"{"type":"error","message":"not JSON"}"#),
            Some(KeeperEvent::Error {
                message: "not JSON".to_string()
            })
        );
        assert_eq!(
            decode_event(r#"{"type":"ok","op":"say"}"#),
            Some(KeeperEvent::Ok {
                op: "say".to_string()
            })
        );
    }

    /// An upstream addition must not break the seam.
    #[test]
    fn an_unknown_shape_is_kept_rather_than_swallowed() {
        match decode_event(r#"{"type":"presence","op":"invite","by":"x"}"#) {
            Some(KeeperEvent::Other { kind }) => assert_eq!(kind, "presence"),
            other => panic!("expected Other, got {other:?}"),
        }
    }

    #[test]
    fn blank_and_non_json_lines_are_ignored_not_fatal() {
        assert_eq!(decode_event(""), None);
        assert_eq!(decode_event("   "), None);
        assert_eq!(decode_event("[kithmoot-agent] connecting..."), None);
    }

    #[test]
    fn commands_encode_as_one_json_line_each() {
        let say = encode_command(&KeeperCommand::Say {
            text: "hello".to_string(),
        });
        assert!(say.ends_with('\n'));
        assert_eq!(say.matches('\n').count(), 1);
        assert_eq!(say.trim(), r#"{"op":"say","text":"hello"}"#);
        assert_eq!(
            encode_command(&KeeperCommand::Roster).trim(),
            r#"{"op":"roster"}"#
        );
        assert_eq!(
            encode_command(&KeeperCommand::Leave).trim(),
            r#"{"op":"leave"}"#
        );
    }

    /// A line with a newline in it would otherwise split into two commands and
    /// desynchronise the seam. serde escapes it; this test says so on purpose,
    /// because it is the kind of thing a future "optimisation" removes.
    #[test]
    fn a_newline_in_text_cannot_split_a_command() {
        let cmd = encode_command(&KeeperCommand::Say {
            text: "one\ntwo".to_string(),
        });
        assert_eq!(cmd.matches('\n').count(), 1, "text newline was not escaped");
        assert!(cmd.contains("one\\ntwo"));
    }

    // ─── retry policy ───

    #[test]
    fn a_closed_room_is_never_retried() {
        assert!(!RoomError::Closed.is_retryable());
        assert!(!RoomError::Link(LinkError::AgentsNotOwned).is_retryable());
        assert!(!RoomError::Relays(RelayError::Empty).is_retryable());
        assert!(RoomError::NoReady.is_retryable());
        assert!(RoomError::Failed("pipe broke".to_string()).is_retryable());
    }

    // ─── mirroring ───

    use crate::comms::{CommsLevel, Party, Tier};

    /// A test double for the seam, so the mirroring rules are tested with no
    /// Node, no relays and no network — which is the reason the trait exists.
    #[derive(Default)]
    struct FakeRoom {
        posted: Vec<String>,
        inbox: Vec<InboundLine>,
        members: Vec<RosterMember>,
        started: bool,
    }

    impl WorldRoom for FakeRoom {
        fn start(&mut self, cfg: &RoomConfig) -> Result<(), RoomError> {
            lint_relays(&cfg.relays).map_err(RoomError::Relays)?;
            self.started = true;
            Ok(())
        }
        fn post(&mut self, line: &OutboundLine) -> Result<(), RoomError> {
            self.posted.push(line.text.clone());
            Ok(())
        }
        fn poll(&mut self) -> Vec<InboundLine> {
            std::mem::take(&mut self.inbox)
        }
        fn members(&self) -> Vec<RosterMember> {
            self.members.clone()
        }
        fn stop(&mut self) -> Result<(), RoomError> {
            self.started = false;
            Ok(())
        }
    }

    /// A family room: the members are kin, so the child's chat is mirrored.
    #[test]
    fn a_child_on_approved_is_mirrored_to_a_family_room() {
        let child = Party::at(CommsLevel::Approved);
        assert!(should_mirror_out(child, &[Tier::Kin, Tier::Kin]));
    }

    /// A crew room: the members are the host's acquaintances, strangers to the
    /// child, so the child's chat does NOT leave the world.
    #[test]
    fn a_child_on_approved_is_not_mirrored_to_a_crew_room() {
        let child = Party::at(CommsLevel::Approved);
        assert!(!should_mirror_out(child, &[Tier::Stranger, Tier::Stranger]));
    }

    /// Ken is hear-only in the room too: recognising a room member does not
    /// give the child a way to speak into that room.
    #[test]
    fn kenning_a_room_member_does_not_mirror_the_child_out() {
        let child = Party::at(CommsLevel::Approved);
        assert!(!should_mirror_out(child, &[Tier::Ken]));
    }

    /// One kin member is enough — a mixed room still carries the child's line,
    /// because the child may speak to that person.
    #[test]
    fn one_kin_member_is_enough_to_mirror() {
        let child = Party::at(CommsLevel::Approved);
        assert!(should_mirror_out(child, &[Tier::Stranger, Tier::Kin]));
    }

    #[test]
    fn a_blocked_player_is_never_mirrored_anywhere() {
        let blocked = Party::at(CommsLevel::Blocked);
        assert!(!should_mirror_out(blocked, &[Tier::Kin, Tier::Kith, Tier::Ken]));
    }

    #[test]
    fn an_empty_room_mirrors_nothing() {
        let adult = Party::at(CommsLevel::Anyone);
        assert!(!should_mirror_out(adult, &[]));
    }

    /// Inbound: the room cannot bypass a ceiling by being a room.
    #[test]
    fn a_room_line_from_a_stranger_does_not_reach_an_approved_child() {
        let child = Party::at(CommsLevel::Approved);
        assert!(!should_mirror_in(child, Tier::Stranger));
        assert!(should_mirror_in(child, Tier::Kin));
        assert!(should_mirror_in(child, Tier::Ken));
    }

    #[test]
    fn the_fake_room_drives_the_seam_without_node() {
        let mut room = FakeRoom::default();
        let cfg = RoomConfig {
            link: "https://example.org/j/#eyJhIjp7ImFnZW50cyI6Im93bmVkLWJ5LW1lbWJlcnMifX0"
                .to_string(),
            name: "Test".to_string(),
            relays: v(&["wss://nos.lol"]),
        };
        room.start(&cfg).expect("fake room starts");
        room.post(&OutboundLine { text: "hello".to_string() }).unwrap();
        assert_eq!(room.posted, vec!["hello".to_string()]);
        room.inbox.push(InboundLine {
            from: "abc".to_string(),
            name: Some("Mum".to_string()),
            text: "dinner".to_string(),
        });
        assert_eq!(room.poll().len(), 1);
        assert!(room.poll().is_empty(), "poll drains");
        room.stop().unwrap();
    }

    /// The fake refuses the same relay list the real one would.
    #[test]
    fn the_fake_room_enforces_the_relay_lint_too() {
        let mut room = FakeRoom::default();
        let cfg = RoomConfig {
            link: "https://example.org/j/#owned-by-members".to_string(),
            name: "Test".to_string(),
            relays: v(&["wss://relay.trotters.cc"]),
        };
        assert_eq!(
            room.start(&cfg).unwrap_err(),
            RoomError::Relays(RelayError::AllForbidden)
        );
    }
}

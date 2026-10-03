//! World chat permissions — who may say what to whom.
//!
//! This module is the safety property of world chat, and it is deliberately
//! pure: no I/O, no globals, no `self`. Everything here is a free function over
//! plain values, so the whole rule can be tested exhaustively and read in one
//! sitting. Spec: `docs/foundations/2026-09-05-world-chat.md` §2.
//!
//! The one idea worth holding on to: **hearing and speaking are separate
//! rights**. `ken` (one-way recognition — I pinned you, you did not pin me) is
//! hear-only, because pinning somebody must not hand them a channel to you.
//! That is not a policy layered on top of the contacts data; it is the shape of
//! the data. Kenspeckle's `ken` entries carry no shared secret and no
//! reciprocal field, because "recognition is one-directional".
//!
//! Everything in here compiles on both targets on purpose. The web build never
//! calls it (there is no chat on web at all, and `check.sh` enforces that), but
//! a permission rule that is only compiled on one target is a permission rule
//! that is only tested on one target. Hence `cfg_attr(..., allow(dead_code))`
//! rather than a bare `cfg` — see the spec §7.8 for why that distinction has
//! bitten this repo before.

#![cfg_attr(target_arch = "wasm32", allow(dead_code))]

/// How one player has classified another, in their own address book.
///
/// Directional and local: `tier(A → B)` is A's view of B. Nothing consults a
/// global graph, because there is no global graph — that is red line 1.
///
/// `Kin` and `Kith` are mutual upstream by construction (both sides ran a bond
/// ceremony). `Ken` is one-way by construction. The engine does not verify
/// mutuality; it trusts each player's own list for that player's own filtering,
/// which is the only thing it is ever used for.
///
/// Constructed by `contacts::parse_kenspeckle_export` (Phase 4, §3.2) and
/// read into `ServerPlayer.contacts` at join (`hosted_server.rs`,
/// `contacts::load_local_book`); `ServerPlayer::tier_of`'s `Stranger`
/// fallback is what an empty or not-yet-imported book still reads as.
// `serde` so the contacts mirror (`contacts::save_mirror`, online play by
// contact §6) can round-trip a tier as `"kin" | "kith" | "ken" | "stranger"`.
// Deliberately NOT `Ord`: "close enough to play in my world" is
// `matches!(t, Tier::Kin | Tier::Kith)` in `online_admission`, an explicit
// enumeration, not a comparison that a variant reorder could silently change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    /// Family.
    Kin,
    /// Mutually verified acquaintance.
    Kith,
    /// One-way recognition. Hear-only — see the module docs.
    Ken,
    /// No relationship in either direction.
    Stranger,
}

/// Every tier, for exhaustive tests and for iterating in diagnostics.
// BRIDGE: only this module's own tests iterate it today — see `Tier::Kin`.
#[allow(dead_code)]
pub const ALL_TIERS: [Tier; 4] = [Tier::Kin, Tier::Kith, Tier::Ken, Tier::Stranger];

/// What a player is permitted to do at all.
///
/// The ordering is load-bearing: `Blocked < Approved < Anyone`, so "tighten" is
/// `min` and nothing else needs to know the order. `PartialOrd`/`Ord` are
/// derived from declaration order — **do not reorder these variants.**
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CommsLevel {
    /// No chat at all; the UI is hidden.
    ///
    /// Produced by an operator's `--chat-level blocked` (`server_main.rs`,
    /// Phase 3) or a guardian policy entry (`charter::comms_level`).
    Blocked,
    /// Hears kin, kith and anyone they ken. Speaks to kin and kith.
    ///
    /// The fail-closed default everywhere a Charter ceiling is unknown
    /// (`PlayerSlot::new`, `charter::comms_level`'s no-file/no-entry/bad-sig
    /// fallback) — §2.6: a missing guardian record is not consent.
    Approved,
    /// Hears and speaks to everybody in the world.
    Anyone,
}

/// Every level, for exhaustive tests.
// BRIDGE: only this module's own tests iterate it today — see `CommsLevel::Blocked`.
#[allow(dead_code)]
pub const ALL_LEVELS: [CommsLevel; 3] =
    [CommsLevel::Blocked, CommsLevel::Approved, CommsLevel::Anyone];

impl CommsLevel {
    /// Parse an operator flag or policy-file value. Case-insensitive.
    ///
    /// Returns `None` rather than defaulting, so a typo in `--chat-level` is a
    /// startup error and not a silently permissive world. Callers:
    /// `server_main`'s `--chat-level` flag and `charter::level_from_policy`'s
    /// per-subject `comms` value.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "blocked" => Some(CommsLevel::Blocked),
            "approved" => Some(CommsLevel::Approved),
            "anyone" => Some(CommsLevel::Anyone),
            _ => None,
        }
    }

    /// The wire/config spelling. Used by `server_main`'s startup log line.
    pub fn as_str(self) -> &'static str {
        match self {
            CommsLevel::Blocked => "blocked",
            CommsLevel::Approved => "approved",
            CommsLevel::Anyone => "anyone",
        }
    }
}

/// One side of a chat pair, as the rule sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Party {
    /// The effective level: `min(charter ceiling, operator policy)`.
    pub level: CommsLevel,
    /// This player's own inbound narrowing. May only tighten what they hear;
    /// it never affects what they may say. Defaults to `level`.
    pub inbound_filter: CommsLevel,
}

impl Party {
    /// A party with no extra inbound narrowing.
    pub fn at(level: CommsLevel) -> Self {
        Party {
            level,
            inbound_filter: level,
        }
    }

    /// What this party may actually hear: their level, narrowed by their own
    /// filter. Clamped with `min` on the server so a patched client that sends
    /// `Anyone` gains nothing.
    pub fn hear_level(self) -> CommsLevel {
        self.level.min(self.inbound_filter)
    }
}

/// Charter sets the ceiling; the operator may only tighten.
///
/// An operator who sets `Anyone` has not granted anything — they have declined
/// to tighten. Recomputed server-side on every join, never cached across
/// sessions.
/// Called at both join sites via `hosted_server::resolve_join_comms` (§3.3):
/// the local slot and a verified remote join, each combining
/// `charter::comms_level(pubkey)` with `HostedServer.operator_comms`.
pub fn effective_level(charter: CommsLevel, operator: CommsLevel) -> CommsLevel {
    charter.min(operator)
}

/// May a speaker at `level` say something to somebody they see as `tier`?
///
/// Note `Ken => false`: recognising somebody grants no right to speak to them.
pub fn speak_ok(level: CommsLevel, tier: Tier) -> bool {
    match level {
        CommsLevel::Blocked => false,
        CommsLevel::Approved => matches!(tier, Tier::Kin | Tier::Kith),
        CommsLevel::Anyone => true,
    }
}

/// May a listener at `hear_level` hear somebody they see as `tier`?
///
/// Note `Ken => true`: this is the whole point of the tier — a child may follow
/// a host or a well-known builder without handing that person a channel back.
pub fn hear_ok(hear_level: CommsLevel, tier: Tier) -> bool {
    match hear_level {
        CommsLevel::Blocked => false,
        CommsLevel::Approved => matches!(tier, Tier::Kin | Tier::Kith | Tier::Ken),
        CommsLevel::Anyone => true,
    }
}

/// The rule. A line from `speaker` reaches `listener` only if the speaker may
/// speak to them **and** the listener may hear the speaker.
///
/// `speaker_sees_listener` is the speaker's own classification of the listener;
/// `listener_sees_speaker` is the listener's own classification of the speaker.
/// They are independent, and for `Ken` they are routinely asymmetric — that
/// asymmetry is the feature.
///
/// Evaluated server-side, once per recipient. Never on the client: a client
/// that filters is a client that can be patched not to.
pub fn delivers(
    speaker: Party,
    speaker_sees_listener: Tier,
    listener: Party,
    listener_sees_speaker: Tier,
) -> bool {
    speak_ok(speaker.level, speaker_sees_listener)
        && hear_ok(listener.hear_level(), listener_sees_speaker)
}

// ─── Sanitising ───

/// Maximum chat line, in bytes.
///
/// 256, not KithMoot's 2 000: this is a line in a HUD over a game, not a
/// document. It also caps a full-rate sender at roughly 7.7 KB/min.
pub const MAX_CHAT_TEXT_LEN: usize = 256;

/// Why a line was refused. Every one of these is reported back to the sender —
/// a chat that silently eats messages is a chat nobody trusts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatReject {
    /// Empty, or nothing but whitespace.
    Empty,
    /// Over `MAX_CHAT_TEXT_LEN` bytes.
    TooLong,
    /// Contains a control character.
    ControlChar,
}

impl ChatReject {
    /// The line shown to the sender. Plain, and says which rule was broken.
    pub fn message(self) -> &'static str {
        match self {
            ChatReject::Empty => "Nothing to say — type something first.",
            ChatReject::TooLong => "Too long — chat lines are 256 characters at most.",
            ChatReject::ControlChar => "That line had characters chat can't carry.",
        }
    }
}

/// Check and normalise a chat line.
///
/// **Rejects rather than truncates.** A truncated sentence is a changed
/// sentence, and a player should be told their line did not go rather than have
/// it arrive altered. This follows the `player_name` hardening at ingress
/// (`hosted_server.rs`) rather than the truncating `sanitize_folder_name`
/// style — the two exist for different reasons.
///
/// The only normalisation is trimming the ends, which cannot change meaning.
pub fn sanitize_chat_text(raw: &str) -> Result<String, ChatReject> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ChatReject::Empty);
    }
    if trimmed.len() > MAX_CHAT_TEXT_LEN {
        return Err(ChatReject::TooLong);
    }
    if trimmed.chars().any(char::is_control) {
        return Err(ChatReject::ControlChar);
    }
    Ok(trimmed.to_string())
}

// ─── Rate limiting ───

/// Lines per minute, per player.
///
/// 30 matches KithMoot's own `MAX_CHAT_MESSAGES_PER_MINUTE`, so a compliant
/// game cannot flood a mirrored room and vice versa. If one of these numbers
/// moves, move the other.
pub const CHAT_RATE_PER_MIN: u32 = 30;

/// Server ticks per second (the 20 TPS loop).
const TICKS_PER_SEC: u64 = 20;

/// Ticks to earn one token back: 60s at 20 TPS, spread over 30 tokens.
const TICKS_PER_TOKEN: u64 = (60 * TICKS_PER_SEC) / CHAT_RATE_PER_MIN as u64;

/// A per-player token bucket, driven by the server tick rather than wall clock
/// so it is deterministic and testable.
///
/// Checked **before** the tier rule: an over-rate line should cost no
/// permission evaluation.
#[derive(Clone, Copy, Debug)]
pub struct RateLimiter {
    tokens: u32,
    last_tick: u64,
    /// True once we have told this player they are rate-limited, so we say it
    /// once rather than thirty times — telling somebody they are flooding, by
    /// flooding them, helps nobody.
    warned: bool,
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl RateLimiter {
    pub fn new() -> Self {
        RateLimiter {
            tokens: CHAT_RATE_PER_MIN,
            last_tick: 0,
            warned: false,
        }
    }

    /// Refill for elapsed ticks, then try to spend one token.
    ///
    /// Returns `true` if the line may proceed.
    pub fn allow(&mut self, now_tick: u64) -> bool {
        let elapsed = now_tick.saturating_sub(self.last_tick);
        let earned = elapsed / TICKS_PER_TOKEN;
        if earned > 0 {
            self.tokens = (self.tokens as u64 + earned).min(CHAT_RATE_PER_MIN as u64) as u32;
            // Keep the remainder so a steady drip of sub-token intervals still
            // accumulates, rather than being rounded away on every call.
            self.last_tick += earned * TICKS_PER_TOKEN;
        }
        if self.tokens > 0 {
            self.tokens -= 1;
            self.warned = false;
            true
        } else {
            false
        }
    }

    /// True exactly once per run of refusals — use it to decide whether to send
    /// the "you're going too fast" line.
    pub fn should_warn(&mut self) -> bool {
        if self.warned {
            false
        } else {
            self.warned = true;
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── speak_ok: the full 3 × 4 table, written out rather than generated,
    // so a change to the rule shows up as a changed expectation. ───

    #[test]
    fn speak_blocked_says_nothing_to_anyone() {
        for tier in ALL_TIERS {
            assert!(!speak_ok(CommsLevel::Blocked, tier), "blocked spoke to {tier:?}");
        }
    }

    #[test]
    fn speak_approved_reaches_kin_and_kith_only() {
        assert!(speak_ok(CommsLevel::Approved, Tier::Kin));
        assert!(speak_ok(CommsLevel::Approved, Tier::Kith));
        assert!(!speak_ok(CommsLevel::Approved, Tier::Ken));
        assert!(!speak_ok(CommsLevel::Approved, Tier::Stranger));
    }

    #[test]
    fn speak_anyone_reaches_everyone() {
        for tier in ALL_TIERS {
            assert!(speak_ok(CommsLevel::Anyone, tier), "anyone blocked on {tier:?}");
        }
    }

    // ─── hear_ok: the full 3 × 4 table. ───

    #[test]
    fn hear_blocked_hears_nobody() {
        for tier in ALL_TIERS {
            assert!(!hear_ok(CommsLevel::Blocked, tier), "blocked heard {tier:?}");
        }
    }

    #[test]
    fn hear_approved_hears_kin_kith_and_ken() {
        assert!(hear_ok(CommsLevel::Approved, Tier::Kin));
        assert!(hear_ok(CommsLevel::Approved, Tier::Kith));
        assert!(hear_ok(CommsLevel::Approved, Tier::Ken));
        assert!(!hear_ok(CommsLevel::Approved, Tier::Stranger));
    }

    #[test]
    fn hear_anyone_hears_everyone() {
        for tier in ALL_TIERS {
            assert!(hear_ok(CommsLevel::Anyone, tier), "anyone deaf to {tier:?}");
        }
    }

    /// The asymmetry that gives `ken` its reason to exist: at `Approved` you
    /// hear somebody you ken, and you may not speak to them.
    #[test]
    fn ken_is_hear_only() {
        assert!(hear_ok(CommsLevel::Approved, Tier::Ken));
        assert!(!speak_ok(CommsLevel::Approved, Tier::Ken));
    }

    // ─── The two safety properties, named as their own tests so a regression
    // reads as "you broke a safety property", not "a table changed". ───

    /// An `Anyone` speaker does not reach an `Approved` listener who has not
    /// ken'd them. The child is protected regardless of who is speaking.
    #[test]
    fn safety_stranger_on_anyone_cannot_reach_an_approved_child() {
        let stranger = Party::at(CommsLevel::Anyone);
        let child = Party::at(CommsLevel::Approved);
        assert!(!delivers(stranger, Tier::Stranger, child, Tier::Stranger));
    }

    /// An `Approved` speaker does not reach a stranger even if that stranger is
    /// on `Anyone`. A restricted child cannot be drawn into talking to the room.
    #[test]
    fn safety_approved_child_cannot_be_drawn_into_speaking_to_a_stranger() {
        let child = Party::at(CommsLevel::Approved);
        let stranger = Party::at(CommsLevel::Anyone);
        assert!(!delivers(child, Tier::Stranger, stranger, Tier::Stranger));
    }

    #[test]
    fn approved_child_and_kin_talk_both_ways() {
        let child = Party::at(CommsLevel::Approved);
        let parent = Party::at(CommsLevel::Anyone);
        assert!(delivers(child, Tier::Kin, parent, Tier::Kin));
        assert!(delivers(parent, Tier::Kin, child, Tier::Kin));
    }

    /// The one-way case end to end: a child kens the host, so the host's lines
    /// arrive; the child's lines do not go back.
    #[test]
    fn kenned_host_is_heard_but_not_answered() {
        let host = Party::at(CommsLevel::Anyone);
        let child = Party::at(CommsLevel::Approved);
        // Host speaks to the room; the child has ken'd them, so it lands.
        assert!(delivers(host, Tier::Stranger, child, Tier::Ken));
        // The child replies; they may not speak to somebody they merely ken.
        assert!(!delivers(child, Tier::Ken, host, Tier::Stranger));
    }

    // ─── effective_level: the full 3 × 3 table. ───

    #[test]
    fn effective_level_full_table() {
        use CommsLevel::*;
        let cases = [
            (Blocked, Blocked, Blocked),
            (Blocked, Approved, Blocked),
            (Blocked, Anyone, Blocked),
            (Approved, Blocked, Blocked),
            (Approved, Approved, Approved),
            (Approved, Anyone, Approved),
            (Anyone, Blocked, Blocked),
            (Anyone, Approved, Approved),
            (Anyone, Anyone, Anyone),
        ];
        for (charter, operator, want) in cases {
            assert_eq!(
                effective_level(charter, operator),
                want,
                "charter {charter:?} + operator {operator:?}"
            );
        }
    }

    /// The property the table encodes: an operator can only ever tighten.
    #[test]
    fn operator_can_never_loosen_a_charter_ceiling() {
        for charter in ALL_LEVELS {
            for operator in ALL_LEVELS {
                assert!(
                    effective_level(charter, operator) <= charter,
                    "operator {operator:?} loosened charter {charter:?}"
                );
            }
        }
    }

    // ─── inbound filter ───

    #[test]
    fn inbound_filter_narrows_hearing_but_not_speaking() {
        let host = Party {
            level: CommsLevel::Anyone,
            inbound_filter: CommsLevel::Approved,
        };
        let stranger = Party::at(CommsLevel::Anyone);
        // The host no longer hears strangers...
        assert!(!delivers(stranger, Tier::Stranger, host, Tier::Stranger));
        // ...but everyone who ken'd the host still hears the host.
        let follower = Party::at(CommsLevel::Approved);
        assert!(delivers(host, Tier::Stranger, follower, Tier::Ken));
    }

    #[test]
    fn inbound_filter_cannot_widen() {
        let child = Party {
            level: CommsLevel::Approved,
            // A patched client asking for more than it is allowed.
            inbound_filter: CommsLevel::Anyone,
        };
        assert_eq!(child.hear_level(), CommsLevel::Approved);
        let stranger = Party::at(CommsLevel::Anyone);
        assert!(!delivers(stranger, Tier::Stranger, child, Tier::Stranger));
    }

    // ─── parsing ───

    #[test]
    fn level_parse_round_trips_and_rejects_junk() {
        for level in ALL_LEVELS {
            assert_eq!(CommsLevel::parse(level.as_str()), Some(level));
        }
        assert_eq!(CommsLevel::parse("  ANYONE "), Some(CommsLevel::Anyone));
        assert_eq!(CommsLevel::parse("everyone"), None);
        assert_eq!(CommsLevel::parse(""), None);
    }

    // ─── sanitiser ───

    #[test]
    fn sanitize_trims_and_keeps_ordinary_text() {
        assert_eq!(sanitize_chat_text("  hello  ").unwrap(), "hello");
        assert_eq!(sanitize_chat_text("a b\tc").unwrap_err(), ChatReject::ControlChar);
        assert_eq!(sanitize_chat_text("emoji ok 🪓").unwrap(), "emoji ok 🪓");
    }

    #[test]
    fn sanitize_rejects_empty_and_whitespace_only() {
        assert_eq!(sanitize_chat_text("").unwrap_err(), ChatReject::Empty);
        assert_eq!(sanitize_chat_text(" ").unwrap_err(), ChatReject::Empty);
        // A tab is whitespace, so a line of only tabs trims to nothing and is
        // Empty — not ControlChar. The control-char rule is about the interior.
        assert_eq!(sanitize_chat_text("   \t ").unwrap_err(), ChatReject::Empty);
    }

    #[test]
    fn sanitize_rejects_control_characters() {
        assert_eq!(
            sanitize_chat_text("hello\nworld").unwrap_err(),
            ChatReject::ControlChar
        );
        assert_eq!(
            sanitize_chat_text("hello\0world").unwrap_err(),
            ChatReject::ControlChar
        );
    }

    /// The boundary, and the fact that it is *bytes* — a multi-byte character
    /// must not let a sender past the cap.
    #[test]
    fn sanitize_length_boundary_is_bytes_not_chars() {
        let at_cap = "a".repeat(MAX_CHAT_TEXT_LEN);
        assert!(sanitize_chat_text(&at_cap).is_ok());
        let over = "a".repeat(MAX_CHAT_TEXT_LEN + 1);
        assert_eq!(sanitize_chat_text(&over).unwrap_err(), ChatReject::TooLong);
        // 128 × 2-byte chars = 256 bytes: fits. One more does not.
        let two_byte = "é".repeat(MAX_CHAT_TEXT_LEN / 2);
        assert_eq!(two_byte.len(), MAX_CHAT_TEXT_LEN);
        assert!(sanitize_chat_text(&two_byte).is_ok());
        let two_byte_over = "é".repeat(MAX_CHAT_TEXT_LEN / 2 + 1);
        assert_eq!(
            sanitize_chat_text(&two_byte_over).unwrap_err(),
            ChatReject::TooLong
        );
    }

    /// Rejection never silently alters a line: whatever comes back out is what
    /// went in, minus surrounding whitespace.
    #[test]
    fn sanitize_never_truncates() {
        let long = "b".repeat(MAX_CHAT_TEXT_LEN + 50);
        assert!(sanitize_chat_text(&long).is_err());
        let fits = "b".repeat(MAX_CHAT_TEXT_LEN);
        assert_eq!(sanitize_chat_text(&fits).unwrap().len(), MAX_CHAT_TEXT_LEN);
    }

    // ─── rate limiter ───

    #[test]
    fn rate_limiter_allows_a_full_minute_then_refuses() {
        let mut rl = RateLimiter::new();
        for i in 0..CHAT_RATE_PER_MIN {
            assert!(rl.allow(0), "refused line {i} within the allowance");
        }
        assert!(!rl.allow(0), "allowed one over the allowance");
    }

    #[test]
    fn rate_limiter_refills_over_time() {
        let mut rl = RateLimiter::new();
        for _ in 0..CHAT_RATE_PER_MIN {
            assert!(rl.allow(0));
        }
        assert!(!rl.allow(0));
        // One token's worth of ticks later, exactly one more line goes.
        assert!(rl.allow(TICKS_PER_TOKEN));
        assert!(!rl.allow(TICKS_PER_TOKEN));
        // A full minute later the bucket is full again, and does not overfill.
        let minute = 60 * TICKS_PER_SEC;
        for i in 0..CHAT_RATE_PER_MIN {
            assert!(rl.allow(TICKS_PER_TOKEN + minute), "refused line {i} after refill");
        }
        assert!(!rl.allow(TICKS_PER_TOKEN + minute), "bucket overfilled");
    }

    /// Sub-token intervals must accumulate rather than being rounded away on
    /// every call — otherwise a player polling faster than the refill rate
    /// never earns anything back.
    #[test]
    fn rate_limiter_accumulates_partial_intervals() {
        let mut rl = RateLimiter::new();
        for _ in 0..CHAT_RATE_PER_MIN {
            assert!(rl.allow(0));
        }
        // Ask repeatedly at intervals shorter than one token's worth.
        let step = TICKS_PER_TOKEN / 4;
        for i in 1..4 {
            assert!(!rl.allow(step * i), "earned a token too early at step {i}");
        }
        assert!(rl.allow(step * 4), "partial intervals were rounded away");
    }

    #[test]
    fn rate_limiter_warns_once_per_run_of_refusals() {
        let mut rl = RateLimiter::new();
        for _ in 0..CHAT_RATE_PER_MIN {
            rl.allow(0);
        }
        assert!(!rl.allow(0));
        assert!(rl.should_warn(), "did not warn on the first refusal");
        assert!(!rl.should_warn(), "warned twice in one run");
        // Once a line goes through again, a later flood warns afresh.
        assert!(rl.allow(60 * TICKS_PER_SEC));
        for _ in 0..CHAT_RATE_PER_MIN {
            rl.allow(60 * TICKS_PER_SEC);
        }
        assert!(!rl.allow(60 * TICKS_PER_SEC));
        assert!(rl.should_warn(), "did not warn again after recovering");
    }
}

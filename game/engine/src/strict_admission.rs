//! C3d — Strict inventory admission at join (Decision 10 as amended by the
//! obligation carrier, OC v88).
//!
//! A Strict world admits a joiner only when its record proves it carries no
//! correction obligation: no record, or a SIDECar v2 record whose history is
//! `Complete` with no owed entry (`JoinerSave::obligations_proven_zero`). An
//! unreadable record, a v1 record (`Incomplete(LegacyV1)`), an overflowed one
//! (`Incomplete(Overflow)`) or any owed entry is refused before `JoinAccept`
//! with a concrete reason. The refusal settles nothing, writes nothing off
//! and infers nothing from slots: the record's bytes stay as they are.

use crate::joiner_save::{IncompleteCause, JoinerSave, ObligationHistory};

/// Why a Strict world refused a join. Carried to the player as the
/// `JoinReject` reason ([`Self::reason`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrictJoinRefusal {
    /// The record is complete but still owes units of `entries` items.
    Owed { entries: usize },
    /// The record (or, for an unreadable one, this join's session) cannot
    /// prove what it owes.
    Incomplete(IncompleteCause),
    /// C3d run 5 (run-4 review LOW-4) — a `--no-lend` host's owning server
    /// gained a local player after Strict was selected: it mirrors the
    /// host's copy of the world over every commit, so it can't admit one.
    NoLendHost,
}

impl StrictJoinRefusal {
    /// The sentence the refused player reads (UK English).
    pub fn reason(&self) -> String {
        match self {
            StrictJoinRefusal::Owed { entries } => format!(
                "This world checks inventories strictly, and your save here still owes items from an earlier correction ({entries} kind{}). Ask whoever runs this world about your save.",
                if *entries == 1 { "" } else { "s" }
            ),
            StrictJoinRefusal::Incomplete(IncompleteCause::LegacyV1) => "This world checks inventories strictly, and your save here is from an older version that didn't record owed items. Ask whoever runs this world about your save.".to_string(),
            StrictJoinRefusal::Incomplete(IncompleteCause::Overflow) => "This world checks inventories strictly, and your save here lost track of some owed items. Ask whoever runs this world about your save.".to_string(),
            StrictJoinRefusal::Incomplete(IncompleteCause::Unreadable) => "This world checks inventories strictly, and your save here couldn't be read. Ask whoever runs this world about your save.".to_string(),
            StrictJoinRefusal::NoLendHost => "This world checks inventories strictly, but the way it is being hosted can't support that, so it can't let you in. Ask whoever runs this world.".to_string(),
        }
    }
}

/// Why a joined Strict session was closed while recovery and replay are not
/// wired yet (C3d). Every Strict teardown goes through ONE function
/// (`HostedServer::strict_bridge_teardown`), which sends [`Self::reason`] as
/// the `JoinReject` reason the player reads (UK English, no internal terms).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrictTeardown {
    /// The world couldn't confirm the player's last change: a definitive
    /// refusal, a contested shared container, a stale world. The fate row
    /// says which; the correction is this teardown until recovery lands.
    Unconfirmed,
    /// A cap or an uncertain source/fit: the world fell behind checking the
    /// player's changes. No fate row.
    FellBehind,
    /// The game sent a change in a form a Strict world doesn't take (an
    /// untyped legacy packet, an edit on a movement packet).
    Untyped,
    /// The strict check found an obligation it can't account for
    /// (Decision 10 invariant breach).
    Obligation,
    /// Something unreadable, foreign or conflicting arrived.
    Unreadable,
    /// C3d run 13 (run-12 review LOW-2) — the server's OWN copy of the
    /// player's window holds a stack the strict check can't represent
    /// exactly: a `Replace` would ship the same window back, so a
    /// correction would loop. Refused, then torn down.
    Unrepresentable,
}

impl StrictTeardown {
    /// The sentence the disconnected player reads (UK English).
    pub fn reason(self) -> &'static str {
        match self {
            StrictTeardown::Unconfirmed => "This world checks inventories strictly and couldn't confirm your last change, so you've been disconnected. Anything this world saved for you is kept.",
            StrictTeardown::FellBehind => "This world checks inventories strictly and fell behind checking your changes, so you've been disconnected. Anything this world saved for you is kept.",
            StrictTeardown::Untyped => "This world checks inventories strictly, and your game sent a change this world can't take yet, so you've been disconnected. Anything this world saved for you is kept.",
            StrictTeardown::Obligation => "This world's strict inventory check found an obligation it can't account for, so you've been disconnected. Ask whoever runs this world about your save.",
            StrictTeardown::Unreadable => "Your game sent this world something it couldn't read, so you've been disconnected. Anything this world saved for you is kept.",
            StrictTeardown::Unrepresentable => "This world checks inventories strictly and found something in your inventory it can't check exactly, so you've been disconnected. Anything this world saved for you is kept.",
        }
    }
}

/// Admit a Strict join? `saved` is the record `GameServer::load_joiner` just
/// read (none for a guest, a first visit or an unreadable file); `session` is
/// the joining session's history as `load_joiner` left it (it marks an
/// unreadable record `Incomplete(Unreadable)`). Called before the record is
/// restored onto the body, so the record — not the still-empty session fold —
/// is what proves the debt.
pub fn check(saved: Option<&JoinerSave>, session: ObligationHistory) -> Result<(), StrictJoinRefusal> {
    if let ObligationHistory::Incomplete(cause) = session {
        return Err(StrictJoinRefusal::Incomplete(cause));
    }
    let Some(saved) = saved else { return Ok(()) };
    if saved.obligations_proven_zero() {
        return Ok(());
    }
    match saved.history() {
        ObligationHistory::Incomplete(cause) => Err(StrictJoinRefusal::Incomplete(cause)),
        ObligationHistory::Complete => Err(StrictJoinRefusal::Owed {
            entries: saved.debt().map_or(1, |d| d.entries().len().max(1)),
        }),
    }
}

#[cfg(test)]
mod teardown_copy_tests {
    use super::StrictTeardown;
    /// Every teardown sentence is plain UK English with no money word and no
    /// internal term (action, frontier, epoch, receipt, rejoin required).
    #[test]
    fn c3d_strict_teardown_copy_is_plain_and_clean() {
        for cause in [StrictTeardown::Unconfirmed, StrictTeardown::FellBehind, StrictTeardown::Untyped, StrictTeardown::Obligation, StrictTeardown::Unreadable, StrictTeardown::Unrepresentable] {
            let text = cause.reason();
            assert!(crate::copy_lint::banned_in(text, crate::copy_lint::BANNED_MONEY_WORDS).is_empty(), "{cause:?}: {text}");
            for jargon in ["action", "frontier", "epoch", "receipt", "rejoin required", "authoritative", "update your game", "carry on with your inventory"] {
                assert!(!text.to_lowercase().contains(jargon), "{cause:?} reads {jargon:?}: {text}");
            }
            assert!(text.ends_with('.'), "{cause:?}");
        }
        // C3d run 5 — and the join refusal for a host that can't run Strict.
        let text = super::StrictJoinRefusal::NoLendHost.reason();
        assert!(crate::copy_lint::banned_in(&text, crate::copy_lint::BANNED_MONEY_WORDS).is_empty(), "{text}");
        assert!(!text.contains("lend") && text.ends_with('.'), "{text}");
        // C3d run 4 — the client's own Strict leave reads the same way.
        let text = crate::remote_client::STRICT_STRAY_CHANGE_REASON;
        assert!(crate::copy_lint::banned_in(text, crate::copy_lint::BANNED_MONEY_WORDS).is_empty(), "{text}");
        for jargon in ["action", "frontier", "epoch", "receipt", "rejoin required", "authoritative", "legacy", "window op"] {
            assert!(!text.to_lowercase().contains(jargon), "client leave reads {jargon:?}: {text}");
        }
        assert!(text.ends_with('.'));
    }
}

/// C3d run 12 (S1; Decision 19's measurement, survey Q15) — one Strict
/// seat's typed refusals: stale-world (a Drift: the cell changed under the
/// client) and other (a rule or a mismatch). Logged in the departure line
/// beside `PossessionTally`; a measurement, never a cheating count.
///
/// C3d run 15 (run-14 review LOW-2) — the bucket follows the client's
/// `old` (the cell as it says it saw it), so a modified client can steer
/// it: a forged `old` id is a Drift, so `StaleWorld`. Measurement only (no
/// item effect). Classifying from server-side facts (the cell, and the
/// block below it, changed since the server last told this client) waits
/// for S2/S3's capture.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RefusalTally {
    pub stale_world: u32,
    pub other: u32,
}

/// C3d run 13 (run-12 review LOW-3) — at most this many live correction
/// `Replace`s per Strict seat in any [`CORRECTION_WINDOW_TICKS`] window.
pub const MAX_CORRECTIONS_PER_WINDOW: usize = 8;
/// The window [`MAX_CORRECTIONS_PER_WINDOW`] counts over, in server ticks.
pub const CORRECTION_WINDOW_TICKS: u64 = 100;

/// The server ticks of one Strict seat's most recent live correction
/// `Replace`s (at most [`MAX_CORRECTIONS_PER_WINDOW`]).
// BRIDGE: correction rate bound — replace when S3 replay lands
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CorrectionRate {
    recent: std::collections::VecDeque<u64>,
}

impl CorrectionRate {
    /// May one more correction `Replace` go at server tick `now`? Records it
    /// when it may; `false` when [`MAX_CORRECTIONS_PER_WINDOW`] already went
    /// in the last [`CORRECTION_WINDOW_TICKS`] ticks (the caller tears down).
    pub fn admit(&mut self, now: u64) -> bool {
        if self.recent.len() >= MAX_CORRECTIONS_PER_WINDOW {
            let oldest = self.recent.front().copied().unwrap_or(0);
            if now.saturating_sub(oldest) < CORRECTION_WINDOW_TICKS { return false; }
            self.recent.pop_front();
        }
        self.recent.push_back(now);
        true
    }
}

/// Which side of [`RefusalTally`] a refusal counts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefusalKind {
    StaleWorld,
    Rule,
}

impl RefusalTally {
    pub fn note(&mut self, kind: RefusalKind) {
        let n = match kind { RefusalKind::StaleWorld => &mut self.stale_world, RefusalKind::Rule => &mut self.other };
        *n = n.saturating_add(1);
    }
    /// The departure line's addition, or `None` if nothing was refused.
    pub fn summary(&self) -> Option<String> {
        (*self != Self::default()).then(|| format!(
            "Strict refusals: {} stale-world (Drift), {} other (rule or mismatch); not counted as cheating",
            self.stale_world, self.other
        ))
    }
}

#[cfg(test)]
mod correction_rate_tests {
    use super::*;
    /// Run-12 review LOW-3 — eight corrections in any 100-tick window; the
    /// ninth inside it is refused, and one is admitted again once the
    /// oldest of the eight is 100 ticks old.
    #[test]
    fn c3d_correction_rate_admits_eight_per_hundred_ticks() {
        let mut rate = CorrectionRate::default();
        for t in 0..8 { assert!(rate.admit(10 + t), "correction {} of 8", t + 1); }
        assert!(!rate.admit(50), "the ninth inside the window");
        assert!(!rate.admit(109), "still inside: the oldest went at tick 10");
        assert!(rate.admit(110), "the oldest is 100 ticks old");
        assert!(!rate.admit(110), "eight again inside the window: the oldest (tick 11) is 99 ticks old");
    }
}

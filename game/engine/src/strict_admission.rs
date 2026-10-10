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

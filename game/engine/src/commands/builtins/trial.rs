//! `/trial` — launch a ⚡ Race trial (Trials, the Challenge Engine's first
//! family). `/trial` or `/trial list` lists the bundled trials; `/trial <id>`
//! starts one; `/trial off` cancels. The heavy lifting (teleport, finish
//! marker, ghost load, arming the run) is a `StartTrial` side-effect the game
//! loop performs — the command stays GameState-free.

use crate::commands::dispatch::{CommandContext, CommandResult};
use crate::commands::registry::{Command, OpLevel};

pub struct TrialCommand;

impl Command for TrialCommand {
    fn name(&self) -> &'static str {
        "trial"
    }
    fn help(&self) -> &'static str {
        "Race a Trial: beat the clock, then chase your ghost"
    }
    fn usage(&self) -> &'static str {
        "/trial [list | <id> | off]"
    }
    /// Op: a Race teleports the player and places beacons, and a Challenge
    /// can hand out a kit — never for a player in someone else's world, who
    /// runs at `OpLevel::None` (review W3 S2). In their own world the player
    /// is op, so nothing changes there.
    fn min_op_level(&self) -> OpLevel {
        OpLevel::Op
    }
    fn is_cheat(&self) -> bool {
        false
    }
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> CommandResult {
        let arg = args.first().map(|s| s.as_str()).unwrap_or("list");
        match arg {
            "list" | "ls" => {
                ctx.info("⚡ Trials — each tests a different part of the game. /trial <id>:");
                ctx.info("  RACES (chase your ghost):");
                for d in crate::trials::CATALOG {
                    ctx.info(format!("   {} — {}", d.id, d.name));
                }
                ctx.info("  CHALLENGES (do the thing):");
                // The feedback trial rides on /idea: it exists only while that
                // command is visible (the alpha-tester gate).
                let feedback_on = ctx.registry.lookup("idea").is_some();
                for (name, display) in crate::scenario::challenge_listing_visible(feedback_on) {
                    ctx.info(format!("   {name} — {display}"));
                }
                CommandResult::Silent
            }
            "off" | "stop" | "cancel" => CommandResult::StopTrial,
            id => {
                if crate::trials::find_def(id).is_some() {
                    // A ⚡ Race (the trials.rs ghost mechanic).
                    CommandResult::StartTrial(id.to_string())
                } else if id == crate::scenario::FEEDBACK_TRIAL
                    && ctx.registry.lookup("idea").is_none()
                {
                    // Gated off: answer as for any unknown trial.
                    CommandResult::Error(format!("Unknown trial '{id}'. Try /trial list."))
                } else if let Some(def) = crate::scenario::named_builtin_def(id) {
                    // An objective challenge — reuse the scenario runner (kit,
                    // event tracking, HUD, completion) so each trial exercises a
                    // genuinely different system.
                    CommandResult::StartScenario { def }
                } else {
                    CommandResult::Error(format!("Unknown trial '{id}'. Try /trial list."))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_id_starts_unknown_errors() {
        // Pure routing test — no CommandContext needed for the id check.
        assert!(crate::trials::find_def("sprint").is_some());
        assert!(crate::trials::find_def("not-a-trial").is_none());
    }

    #[test]
    fn metadata_is_not_a_cheat() {
        assert!(!TrialCommand.is_cheat());
        assert_eq!(TrialCommand.min_op_level(), OpLevel::Op);
    }
}

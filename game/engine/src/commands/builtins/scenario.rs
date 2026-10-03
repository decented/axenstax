//! `/scenario <name>` — launch a challenge scenario (Goal 1 scenario runner).
//!
//! The command only RESOLVES a name to a `ScenarioDef` and returns
//! `CommandResult::StartScenario { def }`; the game loop (which owns
//! `GameState.scenario`) does the actual provisioning — clears the inventory,
//! gives the kit, sets the runner state, resets tick timing. Goal 1 ships the
//! built-in `test` scenario; Goals 3/4 add their defs (here or via JSON).

use crate::commands::dispatch::{CommandContext, CommandResult};
use crate::commands::registry::Command;

pub struct ScenarioCommand;

impl Command for ScenarioCommand {
    fn name(&self) -> &'static str {
        "scenario"
    }
    /// Player-facing alias: the UI + marketing call these "Experiences" (the
    /// engine term is "scenario"), so `/experience` works too. See the
    /// experience-vs-scenario naming note — players never need the internal word.
    fn aliases(&self) -> &'static [&'static str] {
        &["experience"]
    }
    fn help(&self) -> &'static str {
        "Start an Experience — a challenge or game (e.g. /experience test)"
    }
    fn usage(&self) -> &'static str {
        "/experience <name>   e.g. /experience test   (also: /scenario)"
    }
    fn is_cheat(&self) -> bool {
        // A deliberate mode launcher (like picking a game from a menu), not a
        // survival-integrity cheat — so it doesn't flag the World Integrity
        // Ledger. Scenarios are transient and don't overwrite the survival save.
        false
    }
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> CommandResult {
        let arg = args.first().map(|n| n.to_lowercase());
        match arg.as_deref() {
            // No name (or an explicit `list`) → show the feature-coverage
            // challenges + how to start one. The graphical Phase-6 board (press
            // J) is the primary surface now; this stays the chat equivalent.
            None | Some("list") => {
                ctx.success("Challenges (or press J for the board) — type /scenario <name> to start:".to_string());
                let feedback_on = ctx.registry.lookup("idea").is_some();
                for (name, display) in crate::scenario::challenge_listing_visible(feedback_on) {
                    ctx.success(format!("  /scenario {name}  —  {display}"));
                }
                CommandResult::Silent
            }
            Some(name) => match crate::scenario::named_builtin_def(name)
                // The feedback trial rides on /idea (alpha-tester gate): while
                // that command is hidden it is "unknown" here too.
                .filter(|_| {
                    name != crate::scenario::FEEDBACK_TRIAL || ctx.registry.lookup("idea").is_some()
                }) {
                Some(def) => {
                    ctx.success(format!("Starting scenario: {}", def.display_name));
                    CommandResult::StartScenario { def }
                }
                None => {
                    let msg = format!("unknown scenario: {name} (try: /scenario list)");
                    ctx.error(msg.clone());
                    CommandResult::Error(msg)
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::registry::{CommandRegistry, OpLevel};
    use crate::player_slot::PlayerSlot;
    use crate::world::World;

    fn run(args: &[&str]) -> CommandResult {
        let cmd = ScenarioCommand;
        let mut world = World::new();
        let mut time = 0u32;
        let mut step = 1u32;
        let mut creative = false;
        let mut players = vec![PlayerSlot::new(0, glam::Vec3::ZERO, 0.5)];
        let mut log = Vec::new();
        let mut ch = false;
        let mut ev = false;
        let mut ps = false;
        let reg = CommandRegistry::new();
        let mut ctx = CommandContext {
            world: &mut world,
            world_time: &mut time,
            world_time_step: &mut step,
            is_creative: &mut creative, play_mode: &mut { use crate::play_mode::PlayMode; PlayMode::Survival },
            seed: 42,
            world_name: "test",
            players: &mut players,
            player_idx: 0,
            op_level: OpLevel::Op,
            current_tick: 0,
            log: &mut log,
            registry: &reg,
            cheats_used_marker: &mut ch,
            ever_creative_marker: &mut ev,
            pure_survival_broken_marker: &mut ps,
        };
        let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        cmd.execute(&mut ctx, &owned)
    }

    #[test]
    fn known_scenario_returns_start_with_def() {
        let r = run(&["test"]);
        match r {
            CommandResult::StartScenario { def } => {
                assert_eq!(def.kind, crate::scenario::ScenarioKind::Test);
            }
            other => panic!("expected StartScenario, got {other:?}"),
        }
    }

    #[test]
    fn unknown_scenario_errors() {
        assert!(matches!(run(&["nope"]), CommandResult::Error(_)));
    }

    #[test]
    fn no_args_or_list_shows_the_challenge_list() {
        // Discoverability: no-arg (and an explicit `list`) print the challenges
        // instead of erroring, so a tester can find them without the board.
        assert!(matches!(run(&[]), CommandResult::Silent));
        assert!(matches!(run(&["list"]), CommandResult::Silent));
    }

    #[test]
    fn name_is_case_insensitive() {
        // The dispatcher lowercases the command name but not args; the command
        // lowercases the scenario name itself so `/scenario TEST` still works.
        assert!(matches!(run(&["TEST"]), CommandResult::StartScenario { .. }));
    }

    #[test]
    fn experience_is_a_player_facing_alias() {
        assert!(ScenarioCommand.aliases().contains(&"experience"));
        // Both /experience and /scenario resolve to the same command.
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(ScenarioCommand));
        assert!(reg.lookup("experience").is_some(), "/experience must resolve");
        assert!(reg.lookup("scenario").is_some(), "/scenario must still resolve");
        assert_eq!(reg.lookup("experience").unwrap().name(), "scenario");
    }
}

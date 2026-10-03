//! `/buildguide` (#9) — project a captured blueprint from `world.plan_registry`
//! as a build-along ghost guide: `/buildguide <plan>` shows white ghosts where
//! blocks are missing (red where a wrong block sits); they vanish as you build.
//! `/buildguide off` clears it; `/buildguide list` lists available plans.
//!
//! Guided build-along (2026-09-06): `/buildguide mode block|layer|whole` is the
//! chat-side **mode picker** — it re-sequences the *active* guide into
//! one-block-per-step, one-layer-per-step, or the classic whole-plan ghost. A
//! plan can also be laid in a mode directly: `/buildguide <plan> layer`.
//!
//! Not a cheat (a builder aid; places nothing) — `OpLevel::None`. The actual
//! ghost render + the material-list panel read `GameState.build_guide`.

use crate::commands::dispatch::{CommandContext, CommandResult};
use crate::commands::registry::{Command, OpLevel};

pub struct BuildGuideCommand;

impl Command for BuildGuideCommand {
    fn name(&self) -> &'static str {
        "buildguide"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["bg"]
    }
    fn help(&self) -> &'static str {
        "Project a blueprint as a build-along ghost guide"
    }
    fn usage(&self) -> &'static str {
        "/buildguide <plan> [block|layer|whole] | mode <block|layer|whole> | off | list"
    }
    fn min_op_level(&self) -> OpLevel {
        OpLevel::None
    }

    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> CommandResult {
        match args.first().map(|s| s.to_lowercase()).as_deref() {
            None | Some("list") => {
                let names: Vec<String> = ctx
                    .world
                    .plan_registry
                    .entries()
                    .iter()
                    .map(|e| e.plan.name.clone())
                    .collect();
                if names.is_empty() {
                    ctx.success("No blueprints available.".to_string());
                } else {
                    ctx.success(format!("Blueprints: {}", names.join(", ")));
                }
                CommandResult::Success
            }
            Some("off") | Some("stop") => {
                ctx.success("Build-guide off.".to_string());
                CommandResult::StopBuildGuide
            }
            Some("mode") => {
                // The mode picker for an already-laid guide.
                let word = args.get(1).map(|s| s.to_lowercase()).unwrap_or_default();
                match crate::build_steps::StepMode::parse(&word) {
                    Some(mode) => {
                        ctx.success(format!("Build-guide now guiding {}.", mode.label()));
                        CommandResult::SetBuildGuideMode(mode)
                    }
                    None => {
                        let m = "Usage: /buildguide mode <block|layer|whole>".to_string();
                        ctx.error(m.clone());
                        CommandResult::Error(m)
                    }
                }
            }
            _ => {
                // A trailing mode word picks how the guide is stepped;
                // everything before it is the plan name (names may contain
                // spaces, so the mode is only ever the LAST argument).
                let mut parts: Vec<String> = args.to_vec();
                let mode = match parts.last().and_then(|w| {
                    crate::build_steps::StepMode::parse(&w.to_lowercase())
                }) {
                    Some(m) if parts.len() > 1 => {
                        parts.pop();
                        m
                    }
                    // A bare `/buildguide layer` is a mode word with no plan —
                    // leave it as the (unknown) plan name so the error below
                    // tells them to pick a blueprint.
                    _ => crate::build_steps::StepMode::Whole,
                };
                let name = parts.join(" ");
                let origin = match ctx.players.get(ctx.player_idx) {
                    Some(s) => {
                        let p = s.player.pos;
                        [p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32]
                    }
                    None => {
                        let m = "no player".to_string();
                        ctx.error(m.clone());
                        return CommandResult::Error(m);
                    }
                };
                let total = match ctx.world.plan_registry.find_by_plan_name(&name) {
                    Some(entry) => entry.plan.cells.len(),
                    None => {
                        let m = format!("No blueprint named '{name}' (try /buildguide list)");
                        ctx.error(m.clone());
                        return CommandResult::Error(m);
                    }
                };
                ctx.success(format!(
                    "Build-guide '{name}' projected at ({}, {}, {}) — {total} blocks, guiding {}. White ghosts = place here.",
                    origin[0],
                    origin[1],
                    origin[2],
                    mode.label()
                ));
                CommandResult::StartBuildGuide { name, origin, mode }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::registry::{CommandRegistry, OpLevel};
    use crate::play_mode::PlayMode;
    use crate::player_slot::PlayerSlot;
    use crate::world::World;
    use glam::Vec3;

    fn run(world: &mut World, args: &[&str]) -> CommandResult {
        let cmd = BuildGuideCommand;
        let mut t = 0u32;
        let mut s = 4u32;
        let mut creative = false;
        let mut mode = PlayMode::Survival;
        let mut players = vec![PlayerSlot::new(0, Vec3::new(5.0, 64.0, 9.0), 0.5)];
        let mut log = Vec::new();
        let (mut ch, mut ev, mut ps) = (false, false, false);
        let reg = CommandRegistry::new();
        let mut ctx = CommandContext {
            world,
            world_time: &mut t,
            world_time_step: &mut s,
            is_creative: &mut creative,
            play_mode: &mut mode,
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
    fn off_returns_stop() {
        let mut world = World::new();
        assert_eq!(run(&mut world, &["off"]), CommandResult::StopBuildGuide);
    }

    #[test]
    fn list_is_success_even_when_empty() {
        let mut world = World::new(); // fresh world: empty plan registry
        assert_eq!(run(&mut world, &["list"]), CommandResult::Success);
    }

    #[test]
    fn unknown_plan_errors() {
        let mut world = World::new();
        assert!(matches!(
            run(&mut world, &["definitely_not_a_plan"]),
            CommandResult::Error(_)
        ));
    }

    #[test]
    fn mode_picker_sets_the_step_mode() {
        let mut world = World::new();
        assert_eq!(
            run(&mut world, &["mode", "layer"]),
            CommandResult::SetBuildGuideMode(crate::build_steps::StepMode::Layers)
        );
        assert_eq!(
            run(&mut world, &["mode", "block"]),
            CommandResult::SetBuildGuideMode(crate::build_steps::StepMode::BlockByBlock)
        );
        assert!(matches!(
            run(&mut world, &["mode", "sideways"]),
            CommandResult::Error(_)
        ));
    }

    #[test]
    fn a_trailing_mode_word_is_not_part_of_the_plan_name() {
        let mut world = World::new();
        // The plan is still unknown (empty registry), but the error must name
        // the plan WITHOUT the trailing mode word.
        match run(&mut world, &["My", "Hut", "layer"]) {
            CommandResult::Error(m) => assert!(
                m.contains("'My Hut'"),
                "mode word should be stripped from the name, got: {m}"
            ),
            other => panic!("expected an error, got {other:?}"),
        }
    }
}
